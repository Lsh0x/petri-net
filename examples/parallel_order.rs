//! Run with `cargo run --example parallel_order`.
//!
//! Payment and packing are independent tokens for the same order. The core
//! records their progress; the application performs the split and the barrier.
use petri_core::{Definition, Marking, Place, Token, Transition};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Order {
    number: u64,
}

fn complete(net: &Definition, marking: &mut Marking<Order>, transition: &str, token: u64) {
    let transition = net.transition(transition).expect("known transition");
    assert!(net.enabled_transitions(marking).contains(&transition));
    marking
        .move_token(&transition.from, transition.to.clone(), token)
        .expect("token at source");
}

// Application-level AND barrier: both branches must finish for the same order.
fn try_ship(marking: &mut Marking<Order>, order: &Order) -> bool {
    let paid = "paid".to_string();
    let packed = "packed".to_string();
    let payment = marking.tokens_in(&paid).iter().find(|t| &t.data == order);
    let packing = marking.tokens_in(&packed).iter().find(|t| &t.data == order);
    let (Some(payment), Some(packing)) = (payment, packing) else {
        return false;
    };
    let (payment_id, packing_id) = (payment.id, packing.id);
    marking.remove_token(&paid, payment_id).unwrap();
    marking.remove_token(&packed, packing_id).unwrap();
    marking.put_token("shipped".into(), Token::new(4, "shipped", order.clone()));
    true
}

fn main() {
    let net = Definition::new(
        [
            "received",
            "payment_pending",
            "packing_pending",
            "paid",
            "packed",
            "shipped",
        ]
        .into_iter()
        .map(|id| Place::new(id, ()))
        .collect(),
        vec![
            Transition::new("take_payment", "payment_pending", "paid", ()),
            Transition::new("pack_order", "packing_pending", "packed", ()),
        ],
    );
    let order = Order { number: 1001 };
    let mut marking = Marking::new();
    marking.put_token("received".into(), Token::new(1, "received", order.clone()));

    // Application-level split: consume the received order, create two tasks.
    let received = marking.remove_token(&"received".into(), 1).unwrap();
    marking.put_token(
        "payment_pending".into(),
        Token::new(2, "payment_pending", received.data.clone()),
    );
    marking.put_token(
        "packing_pending".into(),
        Token::new(3, "packing_pending", received.data),
    );
    assert_eq!(net.enabled_transitions(&marking).len(), 2);
    println!("Order 1001: payment and packing are both enabled.");

    // Packing can finish before payment. These steps may complete in either order.
    complete(&net, &mut marking, "pack_order", 3);
    assert!(!try_ship(&mut marking, &order));
    assert_eq!(marking.tokens_in(&"packed".into()).len(), 1);
    println!("Packed, but payment is pending: shipping is blocked.");

    complete(&net, &mut marking, "take_payment", 2);
    assert!(try_ship(&mut marking, &order));
    assert!(marking.tokens_in(&"paid".into()).is_empty());
    assert!(marking.tokens_in(&"packed".into()).is_empty());
    assert_eq!(marking.tokens_in(&"shipped".into())[0].data, order);
    assert!(!try_ship(&mut marking, &order));
    println!("Paid and packed: branch tokens consumed, order shipped once.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barrier_does_not_combine_different_orders() {
        let mut marking = Marking::new();
        marking.put_token("paid".into(), Token::new(2, "paid", Order { number: 1 }));
        marking.put_token(
            "packed".into(),
            Token::new(3, "packed", Order { number: 2 }),
        );
        assert!(!try_ship(&mut marking, &Order { number: 1 }));
        assert_eq!(marking.tokens_in(&"paid".into()).len(), 1);
        assert_eq!(marking.tokens_in(&"packed".into()).len(), 1);
    }

    #[test]
    fn payment_can_finish_before_packing() {
        let order = Order { number: 1 };
        let mut marking = Marking::new();
        marking.put_token("paid".into(), Token::new(2, "paid", order.clone()));
        assert!(!try_ship(&mut marking, &order));
        marking.put_token("packed".into(), Token::new(3, "packed", order.clone()));
        assert!(try_ship(&mut marking, &order));
        assert!(!try_ship(&mut marking, &order));
    }
}
