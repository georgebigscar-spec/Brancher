//! Ветки: переключение, создание, удаление. Имена проверяет core
//! ([`brancher_core::branch_name_error`]), поэтому с `-` они не начинаются.

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// `git switch --no-guess <branch>`: только локальная ветка, без неявного создания из remote.
pub fn switch_args(branch: &str) -> Vec<String> {
    owned(&["switch", "--no-guess", branch])
}

/// `git switch --track origin/x`: локальная `x`, следящая за remote-веткой.
pub fn switch_track_args(remote_branch: &str) -> Vec<String> {
    owned(&["switch", "--track", remote_branch])
}

/// Новая ветка от `start` (коммит или ссылка); `checkout` — сразу переключиться на неё.
pub fn create_args(name: &str, start: &str, checkout: bool) -> Vec<String> {
    if checkout {
        owned(&["switch", "-c", name, start])
    } else {
        owned(&["branch", name, start])
    }
}

/// `-D` удаляет и не слитую ветку: core спрашивает подтверждение заранее.
pub fn delete_args(name: &str, force: bool) -> Vec<String> {
    owned(&["branch", if force { "-D" } else { "-d" }, name])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_commands() {
        assert_eq!(switch_args("dev"), ["switch", "--no-guess", "dev"]);
        assert_eq!(switch_track_args("origin/x"), ["switch", "--track", "origin/x"]);
        assert_eq!(create_args("f", "abc", true), ["switch", "-c", "f", "abc"]);
        assert_eq!(create_args("f", "main", false), ["branch", "f", "main"]);
        assert_eq!(delete_args("f", true), ["branch", "-D", "f"]);
    }
}
