//! `git commit` с сообщением из stdin. Хуки работают как в терминале (раздел 1).

pub fn args(amend: bool) -> Vec<String> {
    let mut a = vec![
        "commit".to_owned(),
        "--cleanup=strip".to_owned(),
        "-F".to_owned(),
        "-".to_owned(),
    ];
    if amend {
        a.push("--amend".to_owned());
    }
    a
}
