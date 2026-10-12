//! `git apply` patch из stdin: частичный stage/unstage (`--cached`) и discard
//! (обратное применение к рабочему дереву).

/// `cached` — применять к индексу, иначе к рабочему дереву; `reverse` — откатить patch.
pub fn args(cached: bool, reverse: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["apply".into()];
    if cached {
        a.push("--cached".into());
    }
    a.push("--whitespace=nowarn".into());
    if reverse {
        a.push("--reverse".into());
    }
    a.push("-".into());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_comes_from_stdin() {
        assert_eq!(
            args(true, false),
            ["apply", "--cached", "--whitespace=nowarn", "-"]
        );
        assert_eq!(
            args(true, true),
            ["apply", "--cached", "--whitespace=nowarn", "--reverse", "-"]
        );
        assert_eq!(
            args(false, true),
            ["apply", "--whitespace=nowarn", "--reverse", "-"]
        );
    }
}
