use std::sync::Arc;

use crate::sequence::AtomicSequence;

pub struct Utils;

impl Utils {
    pub fn get_minimum_sequence(sequences: &[Arc<AtomicSequence>]) -> i64 {
        if sequences.is_empty() {
            i64::MAX
        } else {
            sequences.iter().map(|s| s.get()).min().unwrap()
        }
    }

    pub fn get_maximum_sequence(sequences: &[Arc<AtomicSequence>]) -> i64 {
        if sequences.is_empty() {
            i64::MIN
        } else {
            sequences.iter().map(|s| s.get()).max().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::AtomicSequence;

    #[test]
    fn test_get_minimum_sequence_empty() {
        let sequences: Vec<Arc<AtomicSequence>> = Vec::new();
        assert_eq!(Utils::get_minimum_sequence(&sequences), i64::MAX);
    }
}
