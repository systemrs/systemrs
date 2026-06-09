//! M7 slice 2 exit criterion: a method-based model snapshotted mid-run and restored onto
//! a fresh rebuild continues to a byte-identical trajectory (`doc/systemrs-design.md`
//! §6f, `doc/plan-m7.md`).

use systemrs::SimTime;
use systemrs_examples::checkpoint::{run_straight, run_with_checkpoint};

/// Snapshot at a mid-run boundary, restore onto a fresh rebuild, continue — the
/// concatenated trajectory must equal the straight run bit-for-bit.
#[test]
fn checkpoint_continues_byte_identical() {
    let period = SimTime::from_ns(10);
    let step = 7;
    let split = SimTime::from_ns(45);
    let end = SimTime::from_ns(100);

    let reference = run_straight(period, step, end);
    let (before, after) = run_with_checkpoint(period, step, split, end);

    let mut restored = before;
    restored.extend(after);

    assert!(!restored.is_empty(), "the model must produce samples");
    assert!(
        restored.len() > 1,
        "the split must fall mid-run (samples on both sides)"
    );
    assert_eq!(
        restored, reference,
        "restore must continue the timeline byte-identically"
    );
}
