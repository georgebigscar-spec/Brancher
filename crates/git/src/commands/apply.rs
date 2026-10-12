//! `git apply --cached`: частичный stage и unstage по patch из stdin.

/// `reverse` — снять изменения из индекса (unstage).
pub fn args(reverse: bool) -> Vec<String> {
    let mut a: Vec<String> = ["apply", "--cached", "--whitespace=nowarn"]
        .map(String::from)
        .into();
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
        assert_eq!(args(false), ["apply", "--cached", "--whitespace=nowarn", "-"]);
        assert_eq!(
            args(true),
            ["apply", "--cached", "--whitespace=nowarn", "--reverse", "-"]
        );
    }
}
