use petri_core::{Definition, Marking, Place, Token, Transition};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // A flat ordered list acts as an independent reference model. IDs deliberately
    // collide, and moves may target their source or refer to missing tokens.
    #[test]
    fn random_operations_match_reference_model(
        operations in prop::collection::vec((0u8..4, 0u8..5, 0u8..5, 0u64..16, any::<i64>()), 0..256)
    ) {
        let mut marking = Marking::new();
        let mut model: Vec<Token<i64>> = vec![];
        for (kind, source, destination, id, data) in operations {
            let source = source.to_string();
            let destination = destination.to_string();
            match kind {
                0 => {
                    marking.put_token(source.clone(), Token::new(id, "wrong", data));
                    model.push(Token::new(id, source.clone(), data));
                }
                1 => {
                    let expected = model.iter().position(|t| t.id == id && t.place_id == source).map(|pos| model.remove(pos));
                    prop_assert_eq!(marking.remove_token(&source, id), expected);
                }
                2 => {
                    let expected = model.iter().position(|t| t.id == id && t.place_id == source).map(|pos| {
                        let mut token = model.remove(pos);
                        token.place_id = destination.clone();
                        model.push(token.clone());
                        token
                    });
                    prop_assert_eq!(marking.move_token(&source, destination, id), expected);
                }
                _ => {
                    let json = serde_json::to_string(&marking).unwrap();
                    marking = serde_json::from_str(&json).unwrap();
                }
            }
            let mut count = 0;
            for place in 0..5 {
                let place = place.to_string();
                let expected: Vec<_> = model.iter().filter(|t| t.place_id == place).cloned().collect();
                prop_assert_eq!(marking.tokens_in(&place), expected.as_slice());
                count += marking.tokens_in(&place).len();
            }
            prop_assert_eq!(count, model.len());
        }
    }

    #[test]
    fn arbitrary_strings_and_payloads_round_trip(
        source in any::<String>(), destination in any::<String>(), id in any::<u64>(), data in prop::collection::vec(any::<u8>(), 0..512)
    ) {
        let mut m = Marking::new();
        m.put_token(source.clone(), Token::new(id, "wrong", data.clone()));
        let moved = m.move_token(&source, destination.clone(), id).unwrap();
        prop_assert_eq!(moved.id, id);
        prop_assert_eq!(&moved.data, &data);
        prop_assert_eq!(&moved.place_id, &destination);
        let restored: Marking<Vec<u8>> = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        prop_assert_eq!(restored.tokens_in(&destination), &[moved]);
    }

    #[test]
    fn enabling_depends_only_on_source_occupancy(
        occupied in prop::collection::vec(any::<bool>(), 8),
        endpoints in prop::collection::vec((0usize..8, 0usize..8), 0..64)
    ) {
        let net = Definition::new(
            (0..8).map(|i| Place::new(i.to_string(), ())).collect(),
            endpoints.iter().enumerate().map(|(i, (from, to))| Transition::new(i.to_string(), from.to_string(), to.to_string(), ())).collect(),
        );
        let mut m = Marking::new();
        for (i, present) in occupied.iter().enumerate() {
            if *present { m.put_token(i.to_string(), Token::new(i as u64, i.to_string(), ())); }
        }
        let expected: Vec<_> = endpoints.iter().enumerate().filter(|(_, (from, _))| occupied[*from]).map(|(i, _)| i.to_string()).collect();
        let enabled: Vec<_> = net.enabled_transitions(&m).iter().map(|t| t.id.clone()).collect();
        prop_assert_eq!(enabled, expected);
    }

    #[test]
    fn failed_move_preserves_complete_serialized_state(data in prop::collection::vec(any::<i32>(), 0..128)) {
        let mut m = Marking::new();
        for (i, value) in data.into_iter().enumerate() { m.put_token("a".into(), Token::new(i as u64, "a", value)); }
        let before = serde_json::to_value(&m).unwrap();
        prop_assert!(m.move_token(&"a".into(), "b".into(), u64::MAX).is_none());
        prop_assert_eq!(serde_json::to_value(&m).unwrap(), before);
    }
}
