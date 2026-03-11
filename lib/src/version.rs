use std::cmp::{self, Ordering};
use std::collections::HashMap;
use std::fmt;

use serde::ser::{Serialize, Serializer};

#[derive(Clone, Debug)]
pub struct Version {
    parts: Vec<u32>,
    pre_release: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MergedVersion {
    pub issues: HashMap<String, Vec<Version>>,
    pub version_id: Option<String>,
}

impl Version {
    pub fn parse(version_str: &str) -> Option<Version> {
        let without_build = match version_str.find('+') {
            Some(pos) if pos + 1 < version_str.len() => &version_str[..pos],
            Some(_) => return None,
            None => version_str,
        };

        let (numeric_str, pre_release) = match without_build.find('-') {
            Some(pos) => {
                let pre = &without_build[pos + 1..];
                if pre.is_empty() || pre.split('.').any(|id| id.is_empty()) {
                    return None;
                }
                (&without_build[..pos], Some(pre.to_string()))
            }
            None => (without_build, None),
        };

        let parts = numeric_str
            .split('.')
            .map(|p| p.parse::<u32>())
            .collect::<Vec<_>>();
        if parts.iter().any(|p| p.is_err()) {
            return None;
        }
        let mut parts: Vec<u32> = parts.into_iter().map(|p| p.unwrap()).collect();
        while parts.len() < 3 {
            parts.push(0)
        }

        Some(Version { parts, pre_release })
    }

    pub fn major(&self) -> u32 {
        assert!(self.parts.len() >= 3);
        self.parts[0]
    }

    pub fn minor(&self) -> u32 {
        assert!(self.parts.len() >= 3);
        self.parts[1]
    }

    pub fn patch(&self) -> u32 {
        assert!(self.parts.len() >= 3);
        self.parts[2]
    }

    pub fn parts(&self) -> &[u32] {
        self.parts.as_slice()
    }

    pub fn pre_release(&self) -> Option<&str> {
        self.pre_release.as_deref()
    }

    pub fn is_pre_release(&self) -> bool {
        self.pre_release.is_some()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let value = self
            .parts
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(".");
        match self.pre_release {
            Some(ref pre) => write!(f, "{}-{}", value, pre),
            None => write!(f, "{}", value),
        }
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Version) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Version) -> Ordering {
        let min_len = cmp::min(self.parts.len(), other.parts.len());
        for i in 0..min_len {
            let result = self.parts[i].cmp(&other.parts[i]);
            if !result.is_eq() {
                return result;
            }
        }

        // if all else is equal, but one of the Versions has more elements,
        // see if any are non-zero making it greater
        if self.parts.len() != other.parts.len() {
            let longer_parts;
            let nonzero_answer;
            if self.parts.len() > other.parts.len() {
                longer_parts = &self.parts;
                nonzero_answer = Ordering::Greater;
            } else {
                longer_parts = &other.parts;
                nonzero_answer = Ordering::Less;
            }
            for part in longer_parts.iter().skip(min_len) {
                if *part != 0 {
                    return nonzero_answer;
                }
            }
        }

        match (&self.pre_release, &other.pre_release) {
            (None, None) => Ordering::Equal,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(a), Some(b)) => cmp_pre_release_identifiers(a, b),
        }
    }
}

impl Serialize for Version {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

fn cmp_pre_release_identifiers(a: &str, b: &str) -> Ordering {
    let prerelease_parts_a: Vec<&str> = a.split('.').collect();
    let prerelease_parts_b: Vec<&str> = b.split('.').collect();

    for (part_a, part_b) in prerelease_parts_a.iter().zip(prerelease_parts_b.iter()) {
        let result = match (part_a.parse::<u64>(), part_b.parse::<u64>()) {
            (Ok(num_a), Ok(num_b)) => num_a.cmp(&num_b),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => part_a.cmp(part_b),
        };
        if result != Ordering::Equal {
            return result;
        }
    }

    prerelease_parts_a.len().cmp(&prerelease_parts_b.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parse() {
        assert_eq!(
            "1.2.3.4.5",
            Version::parse("1.2.3.4.5").unwrap().to_string()
        );
        assert_eq!("1.2.3.4", Version::parse("1.2.3.4").unwrap().to_string());
        assert_eq!("1.2.3", Version::parse("1.2.3").unwrap().to_string());
        assert_eq!("1.2.0", Version::parse("1.2").unwrap().to_string());
        assert_eq!("1.0.0", Version::parse("1").unwrap().to_string());
    }

    #[test]
    fn test_version_parse_pre_release() {
        let v = Version::parse("2026.3.11-main").unwrap();
        assert_eq!("2026.3.11-main", v.to_string());
        assert_eq!(2026, v.major());
        assert_eq!(3, v.minor());
        assert_eq!(11, v.patch());
        assert_eq!(Some("main"), v.pre_release());
        assert!(v.is_pre_release());

        let v = Version::parse("1.2.3-rc.1").unwrap();
        assert_eq!("1.2.3-rc.1", v.to_string());
        assert_eq!(Some("rc.1"), v.pre_release());

        let v = Version::parse("1.2.3-staging").unwrap();
        assert_eq!("1.2.3-staging", v.to_string());

        let v = Version::parse("1.2.3.4-beta").unwrap();
        assert_eq!("1.2.3.4-beta", v.to_string());
        assert_eq!(&[1, 2, 3, 4], v.parts());
    }

    #[test]
    fn test_version_parse_pre_release_invalid() {
        assert!(Version::parse("1.2.3-").is_none());
        assert!(Version::parse("-beta").is_none());
        assert!(Version::parse("1.2.3-foo..bar").is_none());
        assert!(Version::parse("1.2.3-.foo").is_none());
        assert!(Version::parse("1.2.3-foo.").is_none());
        assert!(Version::parse("1.2.3+").is_none());
    }

    #[test]
    fn test_version_parse_build_metadata_stripped() {
        let v = Version::parse("1.2.3+build.7").unwrap();
        assert_eq!("1.2.3", v.to_string());
        assert!(!v.is_pre_release());

        let v = Version::parse("1.2.3-rc.1+build.123").unwrap();
        assert_eq!("1.2.3-rc.1", v.to_string());
        assert_eq!(Some("rc.1"), v.pre_release());
    }

    #[test]
    fn test_version_parse_no_pre_release() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(None, v.pre_release());
        assert!(!v.is_pre_release());
    }

    #[test]
    fn test_version_equal() {
        assert!(Version::parse("1.0").unwrap() == Version::parse("1.0.0").unwrap());
        assert!(Version::parse("1.0").unwrap() != Version::parse("1.1").unwrap());
    }

    #[test]
    fn test_version_not_equal() {
        assert!(Version::parse("1.0").unwrap() != Version::parse("2.0.0").unwrap());
        assert!(Version::parse("1.0").unwrap() == Version::parse("1.0.0").unwrap());
    }

    #[test]
    fn test_version_pre_release_not_equal_to_release() {
        assert!(Version::parse("1.2.3-main").unwrap() != Version::parse("1.2.3").unwrap());
        assert!(Version::parse("1.2.3-main").unwrap() != Version::parse("1.2.3-staging").unwrap());
        assert!(Version::parse("1.2.3-main").unwrap() == Version::parse("1.2.3-main").unwrap());
    }

    #[test]
    fn test_version_less() {
        // lesser first digit
        assert!(Version::parse("1.0.0.0").unwrap() < Version::parse("2.0.0.0").unwrap());
        // lesser second digit
        assert!(Version::parse("1.0.0.0").unwrap() < Version::parse("1.1.0.0").unwrap());
        // lesser third digit
        assert!(Version::parse("1.0.0.0").unwrap() < Version::parse("1.0.1.0").unwrap());
        // lesser third digit
        assert!(Version::parse("1.0.0.0").unwrap() < Version::parse("1.0.0.1").unwrap());
        // negative test
        assert!(Version::parse("2.0.0.0").unwrap() >= Version::parse("1.0.0.1").unwrap());
    }

    #[test]
    fn test_version_pre_release_less_than_release() {
        assert!(Version::parse("1.2.3-main").unwrap() < Version::parse("1.2.3").unwrap());
        assert!(Version::parse("1.2.3-staging").unwrap() < Version::parse("1.2.3").unwrap());
        assert!(Version::parse("1.2.3").unwrap() > Version::parse("1.2.3-main").unwrap());
    }

    #[test]
    fn test_version_pre_release_ordering() {
        assert!(Version::parse("1.2.3-alpha").unwrap() < Version::parse("1.2.3-beta").unwrap());
        assert!(Version::parse("1.2.3-main").unwrap() < Version::parse("1.2.3-staging").unwrap());
    }

    #[test]
    fn test_version_pre_release_semver_identifier_ordering() {
        assert!(Version::parse("1.2.3-rc.2").unwrap() < Version::parse("1.2.3-rc.10").unwrap());
        assert!(Version::parse("1.2.3-alpha.1").unwrap() < Version::parse("1.2.3-alpha.beta").unwrap());
        assert!(Version::parse("1.2.3-1").unwrap() < Version::parse("1.2.3-alpha").unwrap());
        assert!(Version::parse("1.2.3-alpha").unwrap() < Version::parse("1.2.3-alpha.1").unwrap());
    }

    #[test]
    fn test_version_pre_release_numeric_takes_priority() {
        assert!(Version::parse("1.2.3-main").unwrap() < Version::parse("1.2.4-main").unwrap());
        assert!(Version::parse("1.2.3-main").unwrap() < Version::parse("1.2.4").unwrap());
        assert!(Version::parse("2.0.0-main").unwrap() > Version::parse("1.9.9").unwrap());
    }

    // Impropable in practice, but seems good for completeness
    #[test]
    fn test_version_less_mismatched_parts() {
        assert!(Version::parse("1.0.0.0.0").unwrap() == Version::parse("1.0.0.0").unwrap());
        assert!(Version::parse("1.0.0.0.5").unwrap() > Version::parse("1.0.0.0").unwrap());
        assert!(Version::parse("1.0.0.0.0").unwrap() < Version::parse("1.0.0.0.5").unwrap());
        assert!(Version::parse("1.0.0.0.0.5").unwrap() > Version::parse("1.0.0.0").unwrap());
    }

    #[test]
    fn test_version_less_or_equal() {
        assert!(Version::parse("1.0.0.0").unwrap() <= Version::parse("1.0.0.1").unwrap());
        assert!(Version::parse("1.0.0.0").unwrap() <= Version::parse("1.0.0.0").unwrap());
        assert!(Version::parse("2.0.0.1").unwrap() > Version::parse("1.0.0.0").unwrap());
    }

    #[test]
    fn test_version_greater() {
        assert!(Version::parse("4.8").unwrap() > Version::parse("4.1.2").unwrap());
        assert!(Version::parse("4.8").unwrap() <= Version::parse("4.9").unwrap());
    }

    #[test]
    fn test_version_greater_or_equal() {
        assert!(Version::parse("4.8.1").unwrap() >= Version::parse("4.8").unwrap());
        assert!(Version::parse("4.8").unwrap() >= Version::parse("4.8").unwrap());
        assert!(Version::parse("4.8").unwrap() < Version::parse("4.9").unwrap());
    }

    #[test]
    fn test_version_sort() {
        let mut versions = vec![
            Version::parse("1.2.0.0").unwrap(),
            Version::parse("1.2.3.4").unwrap(),
            Version::parse("1.2.3.0").unwrap(),
            Version::parse("1.0.0.0").unwrap(),
        ];
        versions.sort();

        assert_eq!(
            vec![
                Version::parse("1.0.0.0").unwrap(),
                Version::parse("1.2.0.0").unwrap(),
                Version::parse("1.2.3.0").unwrap(),
                Version::parse("1.2.3.4").unwrap(),
            ],
            versions
        );
    }

    #[test]
    fn test_version_sort_with_pre_release() {
        let mut versions = vec![
            Version::parse("2026.3.11").unwrap(),
            Version::parse("2026.3.11-main").unwrap(),
            Version::parse("2026.3.10").unwrap(),
            Version::parse("2026.3.11-staging").unwrap(),
        ];
        versions.sort();

        assert_eq!(
            vec![
                Version::parse("2026.3.10").unwrap(),
                Version::parse("2026.3.11-main").unwrap(),
                Version::parse("2026.3.11-staging").unwrap(),
                Version::parse("2026.3.11").unwrap(),
            ],
            versions
        );
    }

    #[test]
    fn test_version_max() {
        let versions = &[
            Version::parse("1.0.0.0").unwrap(),
            Version::parse("2.0.0.0").unwrap(),
        ];
        assert_eq!(
            &Version::parse("2.0.0.0").unwrap(),
            versions.iter().max().unwrap()
        );
        assert_eq!(
            &Version::parse("1.0.0.0").unwrap(),
            versions.iter().min().unwrap()
        );
    }

    #[test]
    fn test_version_parts() {
        let v = Version::parse("1.2.3.4").unwrap();
        assert_eq!(1, v.major());
        assert_eq!(2, v.minor());
        assert_eq!(3, v.patch());
        assert_eq!(&[1, 2, 3, 4], v.parts());
    }
}
