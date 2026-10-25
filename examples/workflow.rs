use petri_core::{Definition, Marking, Place, Token, Transition};

fn main() {
    let net = Definition::new(
        vec![Place::new("pending", ()), Place::new("done", ())],
        vec![Transition::new("complete", "pending", "done", ())],
    );
    let mut marking = Marking::new();
    marking.put_token("pending".into(), Token::new(1, "pending", "job"));
    assert_eq!(net.enabled_transitions(&marking).len(), 1);
    marking
        .move_token(&"pending".into(), "done".into(), 1)
        .unwrap();
    assert!(net.enabled_transitions(&marking).is_empty());
}
