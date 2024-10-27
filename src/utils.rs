use std::sync::Arc;

use crate::sequence::AtomicSequence;

pub struct Utils;

impl Utils {
    pub fn get_minimum_sequence(sequences: &[Arc<AtomicSequence>]) -> i64 {
        sequences.iter().map(|s| s.get()).min().unwrap_or_default()
    }

    pub fn get_maximum_sequence(sequences: &[Arc<AtomicSequence>]) -> i64 {
        sequences.iter().map(|s| s.get()).max().unwrap_or_default()
    }
}
