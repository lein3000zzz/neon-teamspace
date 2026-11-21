use neon_domain::settings::{Definition, Key, Registry, checks};

/// The instance's name, shown in the interface, in mail and by `GET /api/v1/meta`.
pub const INSTANCE_NAME: Key<str> = Key::new("instance.name");

/// Every runtime setting the server reads.
///
/// # Panics
///
/// When a definition is invalid. That is a programming error, and a test builds this
/// registry, so it cannot reach a release.
pub fn registry() -> Registry {
    Registry::builder()
        .define(
            Definition::string(
                INSTANCE_NAME,
                "Neon Workspace",
                "The instance's name, shown in the interface and in mail.",
            )
            .check(checks::non_blank)
            .check(checks::max_chars(100)),
        )
        .build()
        .expect("the server's settings are valid")
}

#[cfg(test)]
mod tests {
    use neon_domain::settings::{Pinned, Source, Stored};

    use super::*;

    #[test]
    fn every_setting_resolves_to_its_default() {
        let registry = registry();
        let snapshot = registry.resolve(&Pinned::default(), &Stored::new());

        assert_eq!(snapshot.get(INSTANCE_NAME), "Neon Workspace");
        assert!(snapshot.iter().all(|(_, r)| r.source == Source::Default));
        assert_eq!(registry.check_rules(&snapshot), Ok(()));
    }
}
