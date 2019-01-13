#[derive(Parser)]
#[grammar = "parsers/stdout.pest"] // relative to src
pub struct StdoutParser;

#[cfg(test)]
mod test {
    use parsers::*;
    use pest::Parser;
    use zpool::Zpool;

    #[test]
    fn test_action_good() {
        let one_line = " action: The pool can be imported using its name or numeric identifier.\n";
        parses_to! {
            parser: StdoutParser,
            input: one_line,
            rule: Rule::action,
            tokens: [
                action(0, 72, [
                       action_msg(9,72, [action_good_msg(9,72)])
                ])
            ]
        }
    }

    #[test]
    fn test_action_bad() {
        let two_lines = " action: The pool cannot be imported. Attach the missing\n        \
                         devices and try again.\n";
        parses_to! {
            parser: StdoutParser,
            input: two_lines,
            rule: Rule::action,
            tokens: [
                action(0, 88, [
                       action_msg(9,88,[action_bad_msg(9,88)])
                ])
            ]
        }
    }

    #[test]
    fn test_naked_good() {
        let stdout_valid_two_disks = r#"pool: naked_test
     id: 3364973538352047455
  state: ONLINE
 action: The pool can be imported using its name or numeric identifier.
 config:

        naked_test             ONLINE
          /vdevs/import/vdev0  ONLINE
          /vdevs/import/vdev1  ONLINE
          "#;

        // let pairs = StdoutParser::parse(Rule::zpool_import,
        // stdout_valid_two_disks).unwrap_or_else(|e| panic!("{}", e));
        // println!("{:#?}", pairs);
        parses_to! {
            parser: StdoutParser,
            input: stdout_valid_two_disks,
            rule: Rule::zpool_import,
            tokens: [
                zpool_import(0, 258, [
                    pool_name(0, 17, [name(6,16)]),
                    pool_id(17, 46, [digits(26,45)]),
                    state(46, 62, [state_enum(55, 61)]),
                    action(62, 134, [action_msg(71, 134, [action_good_msg(71, 134)])]),
                    config(134, 143),
                    pool_line(144, 182, [name(152, 162), state_enum(175, 181)]),
                    vdevs(182, 258, [
                        vdev(182,220, [
                            naked_vdev(182, 220, [
                                disk_line(182, 220, [
                                    path(192, 211),
                                    state_enum(213, 219)
                                ])
                            ])
                        ]),
                        vdev(220,258, [
                            naked_vdev(220, 258, [
                                disk_line(220, 258, [
                                    path(230, 249),
                                    state_enum(251, 257)
                                ])
                            ])
                        ])
                    ])
                ])
            ]
        }
        let mut pairs = StdoutParser::parse(Rule::zpool_import, stdout_valid_two_disks)
            .unwrap_or_else(|e| panic!("{}", e));
        let pair = pairs.next().unwrap();
        Zpool::from_pest_pair(pair);
    }

    #[test]
    fn test_naked_bad() {
        let stdout_invalid_two_disks = r#"pool: naked_test
     id: 3364973538352047455
  state: UNAVAIL
 status: One or more devices are missing from the system.
 action: The pool cannot be imported. Attach the missing
        devices and try again.
   see: http://illumos.org/msg/ZFS-8000-6X
 config:

        naked_test             UNAVAIL  missing device
          /vdevs/import/vdev0  ONLINE

        Additional devices are known to be part of this pool, though their
        exact configuration cannot be determined.
        "#;
        // let pairs = StdoutParser::parse(Rule::zpool_import,
        // stdout_invalid_two_disks).unwrap_or_else(|e| panic!("{}", e));
        // println!("{:#?}", pairs);
        parses_to! {
            parser: StdoutParser,
            input: stdout_invalid_two_disks,
            rule: Rule::zpool_import,
            tokens: [
                zpool_import(0, 356, [
                    pool_name(0, 17, [name(6,16)]),
                    pool_id(17, 46, [digits(26,45)]),
                    state(46, 63, [state_enum(55, 62)]),
                    status(63, 121, [action_good_msg(71, 121)]),
                    action(121, 209, [action_msg(130, 209, [action_bad_msg(130, 209)])]),
                    see(209, 252, [url(217, 251)]),
                    config(252, 261),
                    pool_line(262, 317, [name(270, 280), state_enum(293, 300)]),
                    vdevs(317, 356, [
                        vdev(317, 355, [
                            naked_vdev(317, 355, [
                                disk_line(317, 355, [
                                    path(327, 346),
                                    state_enum(348, 354)
                                ])
                            ])
                        ])
                    ])
                ])
            ]
        }

        let mut pairs = StdoutParser::parse(Rule::zpool_import, stdout_invalid_two_disks)
            .unwrap_or_else(|e| panic!("{}", e));
        let pair = pairs.next().unwrap();
        Zpool::from_pest_pair(pair);
    }

    #[test]
    fn test_multiple_import() {
        let stdout = r#"pool: naked_test
     id: 3364973538352047455
  state: ONLINE
 action: The pool can be imported using its name or numeric identifier.
 config:

        naked_test             ONLINE
          /vdevs/import/vdev0  ONLINE
          /vdevs/import/vdev1  ONLINE

     pool: naked_test2
     id: 3364973538352047455
  state: ONLINE
 action: The pool can be imported using its name or numeric identifier.
 config:

        naked_test             ONLINE
          /vdevs/import/vdev0  ONLINE
          /vdevs/import/vdev1  ONLINE
          "#;

        let pairs = StdoutParser::parse(Rule::zpools_import, stdout).unwrap_or_else(|e| panic!("{}", e));
        let mut zpools = pairs.map(|pair| Zpool::from_pest_pair(pair));

        let first = zpools.next().unwrap();
        assert_eq!(first.name(), &String::from("naked_test"));

        let second = zpools.next().unwrap();
        assert_eq!(second.name(), &String::from("naked_test2"));

        let none = zpools.next();

        assert!(none.is_none());
    }
}
