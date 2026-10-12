/// Версия git, например `2.43.0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// NFR-14: Git 2.25.1 или новее.
pub const MIN_GIT_VERSION: GitVersion = GitVersion {
    major: 2,
    minor: 25,
    patch: 1,
};

impl GitVersion {
    /// Разбирает вывод `git --version`:
    /// `git version 2.43.0`, `git version 2.39.3 (Apple Git-145)`, `git version 2.42.0.windows.2`.
    pub fn parse(s: &str) -> Option<Self> {
        let rest = s.trim().strip_prefix("git version ")?;
        let mut nums = rest
            .split(|c: char| !c.is_ascii_digit())
            .take(3)
            .map(|n| n.parse::<u32>());
        let major = nums.next()?.ok()?;
        let minor = nums.next()?.ok()?;
        let patch = nums.next().and_then(Result::ok).unwrap_or(0);
        Some(Self { major, minor, patch })
    }
}

impl std::fmt::Display for GitVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_platform_variants() {
        let v = |a, b, c| {
            Some(GitVersion {
                major: a,
                minor: b,
                patch: c,
            })
        };
        assert_eq!(GitVersion::parse("git version 2.43.0\n"), v(2, 43, 0));
        assert_eq!(
            GitVersion::parse("git version 2.39.3 (Apple Git-145)"),
            v(2, 39, 3)
        );
        assert_eq!(GitVersion::parse("git version 2.42.0.windows.2"), v(2, 42, 0));
        assert_eq!(GitVersion::parse("git version 2.25"), v(2, 25, 0));
        assert_eq!(GitVersion::parse("hg version 6"), None);
    }

    #[test]
    fn compares_with_minimum() {
        assert!(GitVersion::parse("git version 2.25.0").unwrap() < MIN_GIT_VERSION);
        assert!(GitVersion::parse("git version 2.25.1").unwrap() >= MIN_GIT_VERSION);
        assert!(GitVersion::parse("git version 3.0.0").unwrap() > MIN_GIT_VERSION);
    }
}
