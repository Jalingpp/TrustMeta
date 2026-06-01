use crate::digest::Digestible;
use core::iter::FromIterator;
use core::ops::{BitAnd, BitOr, Deref};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub trait SetElement: Digestible + Clone + Send + Sync + Eq + PartialEq + core::hash::Hash {}

impl<T> SetElement for T where
    T: Digestible + Clone + Send + Sync + Eq + PartialEq + core::hash::Hash
{
}

#[derive(Debug, Clone, Eq, PartialEq, Default, Serialize, Deserialize)]
pub struct Set<T: SetElement> {
    pub(crate) inner: HashSet<T>,
}

impl<T: SetElement> Set<T> {
    pub fn new() -> Self {
        Self {
            inner: HashSet::new(),
        }
    }

    pub fn from_vec(input: Vec<T>) -> Self {
        Self::from_iter(input)
    }

    pub fn is_intersected_with(&self, other: &Self) -> bool {
        let (a, b) = if self.len() < other.len() {
            (self, other)
        } else {
            (other, self)
        };
        a.iter().any(|v| b.contains(v))
    }

    pub fn insert(&mut self, element: T) -> bool {
        self.inner.insert(element)
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn delete(&mut self, element: &T) -> bool {
        self.inner.remove(element)
    }

    pub fn intersection(&self, other: &Self) -> Self {
        let mut data = HashSet::new();
        for k in self.iter() {
            if other.contains(k) {
                data.insert(k.clone());
            }
        }
        Self { inner: data }
    }

    pub fn union(&self, other: &Self) -> Self {
        let mut data = self.inner.clone();
        for k in other.iter() {
            data.insert(k.clone());
        }
        Self { inner: data }
    }

    pub fn difference(&self, other: &Self) -> Self {
        let mut data = HashSet::new();
        for k in self.iter() {
            if !other.contains(k) {
                data.insert(k.clone());
            }
        }
        Self { inner: data }
    }
}

impl<T: SetElement> Deref for Set<T> {
    type Target = HashSet<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, T: SetElement> BitOr<&'a Set<T>> for &Set<T> {
    type Output = Set<T>;

    fn bitor(self, other: &'a Set<T>) -> Set<T> {
        let mut data = self.inner.clone();
        for k in other.iter() {
            data.insert(k.clone());
        }
        Set { inner: data }
    }
}

impl<'a, T: SetElement> BitAnd<&'a Set<T>> for &Set<T> {
    type Output = Set<T>;

    fn bitand(self, other: &'a Set<T>) -> Set<T> {
        let mut data = HashSet::new();
        for k in self.iter() {
            if other.contains(k) {
                data.insert(k.clone());
            }
        }
        Set { inner: data }
    }
}

impl<T: SetElement> FromIterator<T> for Set<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let data: HashSet<T> = iter.into_iter().collect();
        Self { inner: data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_intersected_with() {
        let s1 = Set::from_vec(vec![1, 2, 3]);
        let s2 = Set::from_vec(vec![2, 2, 5]);
        let s3 = Set::from_vec(vec![5, 6]);
        assert!(s1.is_intersected_with(&s2));
        assert!(!s1.is_intersected_with(&s3));
    }

    #[test]
    fn test_set_union() {
        let s1 = Set::from_vec(vec![1, 1, 2]);
        let s2 = Set::from_vec(vec![2, 2, 3]);
        let s3 = Set::from_vec(vec![1, 2, 3]);
        assert_eq!(&s1 | &s2, s3);
    }

    #[test]
    fn test_set_intersection() {
        let s1 = Set::from_vec(vec![1, 1, 2]);
        let s2 = Set::from_vec(vec![2, 2, 3]);
        let s3 = Set::from_vec(vec![2]);
        assert_eq!(&s1 & &s2, s3);
    }

    #[test]
    fn test_serde() {
        let s = Set::from_vec(vec![1, 1, 2]);
        let json = serde_json::to_string_pretty(&s).unwrap();
        let bin = bincode::serialize(&s).unwrap();
        assert_eq!(serde_json::from_str::<Set<i32>>(&json).unwrap(), s);
        assert_eq!(bincode::deserialize::<Set<i32>>(&bin[..]).unwrap(), s);
    }
}
