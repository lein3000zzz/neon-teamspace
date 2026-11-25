//! What a workspace may hold: people, storage and agents.
//!
//! Each limit is a whole number where 0 means no limit, as the instance's defaults are
//! settings and settings cannot be unset. A workspace takes the instance's default unless
//! an instance admin overrides it, and an override of 0 lifts the limit for that workspace.
//! A workspace at a limit refuses what would go past it; nothing it holds is ever removed.

/// A limit as set: 0 for none, else the most allowed.
pub type LimitValue = i64;

/// The three limits, as the settings or an override set them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LimitValues {
    /// People in the workspace, guests included.
    pub members: LimitValue,
    pub storage_bytes: LimitValue,
    /// Agents and integrations together, deleted ones left out.
    pub agents: LimitValue,
}

/// A workspace's own limits: each unset follows the instance's default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LimitOverrides {
    pub members: Option<LimitValue>,
    pub storage_bytes: Option<LimitValue>,
    pub agents: Option<LimitValue>,
}

/// The limits that hold for a workspace; `None` is no limit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    pub members: Option<u64>,
    pub storage_bytes: Option<u64>,
    pub agents: Option<u64>,
}

impl Limits {
    /// Each override, or else the default; then 0 is no limit.
    pub fn resolve(overrides: LimitOverrides, defaults: LimitValues) -> Self {
        Self {
            members: cap(overrides.members.unwrap_or(defaults.members)),
            storage_bytes: cap(overrides.storage_bytes.unwrap_or(defaults.storage_bytes)),
            agents: cap(overrides.agents.unwrap_or(defaults.agents)),
        }
    }

    /// Whether one more person fits beside the `members` there are.
    pub fn admits_member(&self, members: i64) -> bool {
        admits(self.members, members)
    }

    /// Whether one more agent or integration fits beside the `agents` there are.
    pub fn admits_agent(&self, agents: i64) -> bool {
        admits(self.agents, agents)
    }
}

fn admits(limit: Option<u64>, count: i64) -> bool {
    limit.is_none_or(|limit| u64::try_from(count).is_ok_and(|count| count < limit))
}

fn cap(value: LimitValue) -> Option<u64> {
    u64::try_from(value).ok().filter(|limit| *limit > 0)
}

/// A limit below 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a limit is a whole number, 0 for none")]
pub struct InvalidLimit;

pub fn check_limit(value: LimitValue) -> Result<LimitValue, InvalidLimit> {
    if value >= 0 {
        Ok(value)
    } else {
        Err(InvalidLimit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_win_and_zero_is_no_limit() {
        let defaults = LimitValues {
            members: 10,
            storage_bytes: 0,
            agents: 3,
        };

        assert_eq!(
            Limits::resolve(LimitOverrides::default(), defaults),
            Limits {
                members: Some(10),
                storage_bytes: None,
                agents: Some(3),
            }
        );
        assert_eq!(
            Limits::resolve(
                LimitOverrides {
                    members: Some(0),
                    storage_bytes: Some(1 << 30),
                    agents: Some(5),
                },
                defaults
            ),
            Limits {
                members: None,
                storage_bytes: Some(1 << 30),
                agents: Some(5),
            }
        );
        assert_eq!(
            Limits::resolve(LimitOverrides::default(), LimitValues::default()),
            Limits::default(),
            "unlimited unless set"
        );
    }

    #[test]
    fn a_member_fits_below_the_limit_only() {
        let three = Limits {
            members: Some(3),
            ..Limits::default()
        };

        assert!(three.admits_member(0));
        assert!(three.admits_member(2));
        assert!(!three.admits_member(3));
        assert!(!three.admits_member(4));
        assert!(Limits::default().admits_member(i64::MAX));
    }

    #[test]
    fn an_agent_fits_below_its_own_limit_only() {
        let two = Limits {
            agents: Some(2),
            members: Some(1),
            ..Limits::default()
        };

        assert!(two.admits_agent(1));
        assert!(!two.admits_agent(2));
        assert!(
            !two.admits_member(1),
            "people count against their own limit"
        );
        assert!(Limits::default().admits_agent(i64::MAX));
    }

    #[test]
    fn limits_are_not_negative() {
        assert_eq!(check_limit(0), Ok(0));
        assert_eq!(check_limit(50), Ok(50));
        assert_eq!(check_limit(-1), Err(InvalidLimit));
    }
}
