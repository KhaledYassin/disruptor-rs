use std::sync::Arc;

use crate::sequence::{AtomicSequence, Sequence};

pub struct Utils;

impl Utils {
    pub fn get_minimum_sequence(sequences: &[Arc<AtomicSequence>], sequence: Sequence) -> i64 {
        let mut minimum = sequence;

        for sequence in sequences {
            let cursor = sequence.get();
            minimum = minimum.min(cursor);
        }

        minimum
    }

    pub fn get_maximum_sequence(sequences: &[Arc<AtomicSequence>]) -> i64 {
        let mut max = 0;

        for sequence in sequences {
            let cursor = sequence.get();
            if cursor > max {
                max = cursor;
            }
        }

        max
    }
}
