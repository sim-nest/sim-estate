use super::*;
fn exercise<T: Table>(book: &Book<T>) {
    let plan = book.intern(&vec!["canonical", "plan"]).unwrap();
    book.append("r", EventKind::Opened { plan: plan.clone() })
        .unwrap();
    book.append("r", EventKind::DispatchIntent).unwrap();
    book.append(
        "r",
        EventKind::Final {
            state: "process-complete/unverified".into(),
        },
    )
    .unwrap();
    let a = book.projection("r").unwrap();
    let b = book.projection("r").unwrap();
    assert_eq!(a, b);
    assert_eq!(a.plan, plan);
}
#[test]
fn projections_rebuild_identically_in_memory() {
    exercise(&Book::new(MemoryTable::default()));
}
#[test]
fn approval_is_single_use() {
    let b = Book::new(MemoryTable::default());
    let u = ApprovalUse {
        plan: Key("p".into()),
        reviewer: "alice".into(),
        consumed_by: "run".into(),
    };
    b.issue_approval("a", &u).unwrap();
    b.consume_approval("a", "run").unwrap();
    assert!(matches!(
        b.consume_approval("a", "run"),
        Err(BookError::Conflict { .. })
    ));
}
#[test]
fn event_head_conflict_preserves_published_event() {
    let t = MemoryTable::default();
    let b = Book::new(t);
    let p = b.intern(&"p").unwrap();
    b.append("r", EventKind::Opened { plan: p }).unwrap();
    let stale = serde_json::to_vec(&Key("sha256:stale".into())).unwrap();
    assert!(matches!(
        b.table
            .compare_and_swap(&Key("run/r/head".into()), Some(&stale), b"x"),
        Err(BookError::Conflict { .. })
    ));
}
