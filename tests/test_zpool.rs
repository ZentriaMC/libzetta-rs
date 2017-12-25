extern crate libzfs;
extern crate tempdir;
extern crate slog_term;
extern crate cavity;
extern crate rand;


use cavity::{Bytes, WriteMode, fill};
use libzfs::slog::*;
use libzfs::zpool::{Disk, TopologyBuilder, Vdev, ZpoolEngine, ZpoolOpen3};
use libzfs::zpool::{FailMode, ZpoolError, ZpoolErrorKind, ZpoolPropertiesWriteBuilder};
use rand::Rng;
use std::fs;

use std::panic;
use std::path::{Path, PathBuf};
static ZPOOL_NAME_PREFIX: &'static str = "tests";


fn get_zpool_name() -> String {
    let mut rng = rand::thread_rng();
    let suffix = rng.gen::<u64>();
    let name = format!("{}-{}", ZPOOL_NAME_PREFIX, suffix);
    name

}
fn setup_vdev<P: AsRef<Path>>(path: P, bytes: &Bytes) -> PathBuf {
    let path = path.as_ref();
    if path.exists() {
        let meta = fs::metadata(&path).unwrap();
        assert!(meta.is_file());
        assert!(!meta.permissions().readonly());
        if (meta.len() as usize) < bytes.as_bytes() {
            let _ = fs::remove_file(&path);
            setup_vdev(path, bytes)
        } else {
            path.into()
        }
    } else {
        let mut f = fs::File::create(path).unwrap();
        fill(bytes.clone(), None, WriteMode::FlushOnce, &mut f).unwrap();
        path.into()
    }
}
fn setup() {
    // Create vdevs if they're missing
    let vdev_dir = Path::new("/vdevs");
    setup_vdev(vdev_dir.join("vdev0"), &Bytes::MegaBytes(64 + 10));
    setup_vdev(vdev_dir.join("vdev1"), &Bytes::MegaBytes(64 + 10));
    setup_vdev(vdev_dir.join("vdev2"), &Bytes::MegaBytes(64 + 10));
    setup_vdev(vdev_dir.join("vdev3"), &Bytes::MegaBytes(1));
}
#[allow(dead_code)]
fn teardown() {
    // no-op
}

fn run_test<T>(test: T)
where
    T: FnOnce() -> () + panic::UnwindSafe,
{
    setup();

    let result = panic::catch_unwind(test);

    teardown();

    result.unwrap();
}
fn get_logger() -> Logger {
    let plain = slog_term::PlainSyncDecorator::new(std::io::stdout());
    Logger::root(slog_term::FullFormat::new(plain)
                     .use_original_order()
                     .build()
                     .fuse(),
                 o!())
}

#[test]
fn create_check_update_delete() {
    run_test(|| {
        let zpool = ZpoolOpen3::with_logger(get_logger());
        let name = get_zpool_name();


        let topo = TopologyBuilder::default()
            .vdev(Vdev::Naked(Disk::File("/vdevs/vdev0".into())))
            .build()
            .unwrap();

        zpool.create(&name, topo, None, None, None).unwrap();

        let result = zpool.exists(&name).unwrap();
        assert!(result);

        let props = zpool.read_properties(&name).unwrap();
        let updated_props = ZpoolPropertiesWriteBuilder::from_props(&props)
            .auto_expand(true)
            .auto_replace(true)
            .comment("Wat")
            .fail_mode(FailMode::Panic)
            .build()
            .unwrap();

        zpool.update_properties(&name, updated_props).unwrap();
        let props = zpool.read_properties(&name).unwrap();
        assert_eq!(true, props.auto_expand);
        assert_eq!(true, props.auto_replace);
        assert_eq!(Some(String::from("Wat")), props.comment);
        assert_eq!(FailMode::Panic, props.fail_mode);

        let updated_props = ZpoolPropertiesWriteBuilder::from_props(&props)
            .comment("Wat")
            .build()
            .unwrap();
        zpool.update_properties(&name, updated_props).unwrap();
        let props = zpool.read_properties(&name).unwrap();
        assert_eq!(true, props.auto_expand);
        assert_eq!(true, props.auto_replace);
        assert_eq!(Some(String::from("Wat")), props.comment);
        assert_eq!(FailMode::Panic, props.fail_mode);

        let updated_props = ZpoolPropertiesWriteBuilder::from_props(&props)
            .comment(String::new())
            .delegation(true)
            .build()
            .unwrap();
        zpool.update_properties(&name, updated_props).unwrap();
        let props = zpool.read_properties(&name).unwrap();
        assert_eq!(None, props.comment);
        assert_eq!(true, props.delegation);

        zpool.destroy(&name, true).unwrap();

        let result = zpool.exists(&name).unwrap();
        assert!(!result);
    })
}

#[test]
fn cmd_not_found() {
    run_test(|| {
        let zpool = ZpoolOpen3::with_cmd("zpool-not-found");
        let name = get_zpool_name();

        let topo = TopologyBuilder::default()
            .vdev(Vdev::Naked(Disk::File("/vdevs/vdev0".into())))
            .build()
            .unwrap();

        let result = zpool.create(&name, topo, None, None, None);
        assert_eq!(ZpoolErrorKind::CmdNotFound, result.unwrap_err().kind());

        let result = zpool.exists("wat");
        assert_eq!(ZpoolErrorKind::CmdNotFound, result.unwrap_err().kind());
    })
}

#[test]
fn reuse_vdev() {
    run_test(|| {
        let zpool = ZpoolOpen3::default();
        let name_1 = get_zpool_name();
        let name_2 = "zpool-tests-fail";
        let vdev_file = "/vdevs/vdev1";

        let topo = TopologyBuilder::default()
            .vdev(Vdev::Naked(Disk::File(vdev_file.into())))
            .build()
            .unwrap();

        let props = ZpoolPropertiesWriteBuilder::default().build().unwrap();

        let result = zpool.create(&name_1, topo.clone(), Some(props), None, None);
        result.unwrap();
        let result = zpool.create(&name_2, topo.clone(), None, None, None);
        let err = result.unwrap_err();
        assert_eq!(ZpoolErrorKind::VdevReuse, err.kind());
        println!("{:?}", &err);
        if let ZpoolError::VdevReuse(vdev, pool) = err {
            assert_eq!(vdev_file, vdev);
            assert_eq!(name_1, pool);
        }
        zpool.destroy(&name_1, true).unwrap();
    });
}
#[test]
fn create_invalid_topo() {
    let zpool = ZpoolOpen3::default();
    let name = get_zpool_name();


    let topo = TopologyBuilder::default()
        .cache(Disk::file("/vdevs/vdev0"))
        .build()
        .unwrap();

    let result = zpool.create(&name, topo, None, None, None);

    let err = result.unwrap_err();
    assert_eq!(ZpoolErrorKind::InvalidTopology, err.kind());
}

#[test]
fn remove_pool_not_found() {
    let zpool = ZpoolOpen3::default();
    let name = get_zpool_name();

    let err = zpool.destroy(&name, true).unwrap_err();

    assert_eq!(ZpoolErrorKind::PoolNotFound, err.kind())
}


#[test]
fn pool_not_found() {
    let zpool = ZpoolOpen3::default();
    let name = get_zpool_name();

    let err = zpool.read_properties(&name).unwrap_err();
    assert_eq!(ZpoolErrorKind::PoolNotFound, err.kind());

    let props = ZpoolPropertiesWriteBuilder::default().build().unwrap();
    let err = zpool.update_properties(&name, props).unwrap_err();
    assert_eq!(ZpoolErrorKind::PoolNotFound, err.kind());
}

#[test]
fn read_args() {
    let zpool = ZpoolOpen3::default();
    let name = get_zpool_name();

    let vdev_path = setup_vdev("/vdevs/vdev4", &Bytes::MegaBytes(64 + 10));
    let topo = TopologyBuilder::default()
        .vdev(Vdev::file(vdev_path))
        .build()
        .unwrap();

    zpool.create(&name, topo, None, None, None).unwrap();

    let props = zpool.read_properties(&name);

    assert!(props.is_ok());
    zpool.destroy(&name, true).unwrap();
}

#[test]
fn create_mount() {
    let zpool = ZpoolOpen3::default();
    let name = get_zpool_name();
    let mut mount_point = PathBuf::from("/tmp");
    mount_point.push(&name);

    let vdev_path = setup_vdev("/vdevs/vdev5", &Bytes::MegaBytes(64 + 10));
    let topo = TopologyBuilder::default()
        .vdev(Vdev::file(vdev_path))
        .build()
        .unwrap();

    assert!(!mount_point.exists());
    let result = zpool.create(&name, topo, None, mount_point.clone(), None);
    result.unwrap();
    assert!(mount_point.exists());
    zpool.destroy(&name, true).unwrap();
}

#[test]
fn create_mount_and_alt_root() {
    let zpool = ZpoolOpen3::default();
    let name = get_zpool_name();
    let mut mount_point = PathBuf::from("/tmp");
    mount_point.push(&name);

    let mut expected = PathBuf::from("/mnt/tmp");
    expected.push(&name);

    let alt_root = PathBuf::from("/mnt");

    let vdev_path = setup_vdev("/vdevs/vdev6", &Bytes::MegaBytes(64 + 10));
    let topo = TopologyBuilder::default()
        .vdev(Vdev::file(vdev_path))
        .build()
        .unwrap();

    let result = zpool.create(&name, topo, None, mount_point.clone(), alt_root.clone());
    result.unwrap();

    let props = zpool.read_properties(&name).unwrap();
    assert_eq!(props.alt_root, Some(PathBuf::from("/mnt")));

    assert!(expected.exists());
    zpool.destroy(&name, true).unwrap();
}
#[test]
fn create_with_props() {
    let zpool = ZpoolOpen3::default();
    let name = get_zpool_name();
    let comment = String::from("this is a comment");

    let alt_root = PathBuf::from("/mnt");
    let vdev_path = setup_vdev("/vdevs/vdev7", &Bytes::MegaBytes(64 + 10));
    let topo = TopologyBuilder::default()
        .vdev(Vdev::file(vdev_path))
        .build()
        .unwrap();

    let props = ZpoolPropertiesWriteBuilder::default()
        .auto_expand(true)
        .comment(comment.clone())
        .fail_mode(FailMode::Panic)
        .build()
        .unwrap();

    zpool.create(&name,
                 topo,
                 props,
                 Some(alt_root.clone()),
                 Some(alt_root.clone()))
         .unwrap();

    let props = zpool.read_properties(&name).unwrap();
    assert_eq!(true, props.auto_expand);
    assert_eq!(FailMode::Panic, props.fail_mode);
    assert_eq!(Some(comment.clone()), props.comment);
    zpool.destroy(&name, true).unwrap();
}
