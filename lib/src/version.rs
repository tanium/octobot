use std::cmp::{self, Ordering};
use std::collections::HashMap;
use std::fmt;

use serde::ser::{Serialize, Serializer};

#[derive(Clone, Debug)]
pub struct Version {
    parts: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MergedVersion {
    pub issues: HashMap<String, Vec<Version>>,
    pub version_id: Option<String>,
}

impl Version {
    pub fn parse(version_str: &str) -> Option<Version> {
        let parts = version_str
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

        Some(Version { parts })
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
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let value = self
            .parts
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(".");
        write!(f, "{}", value)
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
        // Note: this is always going to be at least 3.
        let min_len = cmp::min(self.parts.len(), other.parts.len());
        for i in 0..min_len {
            let result = self.parts[i].cmp(&other.parts[i]);
            if !result.is_eq() {
                return result;
            }
        }

        if self.parts.len() == other.parts.len() {
            return Ordering::Equal;
        }

        // if all else is equal, but one of the Versions has more elements,
        // see if any are non-zero making it greater
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

        Ordering::Equal
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
    fn test_version_equal() {
        assert!(Version::parse("1.0").unwrap() == Version::parse("1.0.0").unwrap());
        assert!(!(Version::parse("1.0").unwrap() == Version::parse("1.1").unwrap()));
    }

    #[test]
    fn test_version_not_equal() {
        assert!(Version::parse("1.0").unwrap() != Version::parse("2.0.0").unwrap());
        assert!(!(Version::parse("1.0").unwrap() != Version::parse("1.0.0").unwrap()));
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
        assert!(!(Version::parse("2.0.0.0").unwrap() < Version::parse("1.0.0.1").unwrap()));
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
        assert!(!(Version::parse("2.0.0.1").unwrap() <= Version::parse("1.0.0.0").unwrap()));
    }

    #[test]
    fn test_version_greater() {
        assert!(Version::parse("4.8").unwrap() > Version::parse("4.1.2").unwrap());
        assert!(!(Version::parse("4.8").unwrap() > Version::parse("4.9").unwrap()));
    }

    #[test]
    fn test_version_greater_or_equal() {
        assert!(Version::parse("4.8.1").unwrap() >= Version::parse("4.8").unwrap());
        assert!(Version::parse("4.8").unwrap() >= Version::parse("4.8").unwrap());
        assert!(!(Version::parse("4.8").unwrap() >= Version::parse("4.9").unwrap()));
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
    fn test_version_max() {
        let versions = vec![
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
