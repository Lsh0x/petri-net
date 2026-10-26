//! Native fork/join, JSON transport and a shared authoritative store.
use petri_core::engine::{Arc, Authenticator, Definition, MemoryStorage, Petri, Token, Transition};
use std::sync::Arc as Shared;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    futures::executor::block_on(async {
        // Numeric places: received=0, payment=1, packing=2, paid=3, packed=4,
        // shipped=5. Numeric transitions: split=0, pay=1, pack=2, ship=3.
        let definition = Definition::new(
            vec![0, 1, 2, 3, 4, 5],
            0,
            vec![
                Transition::new(
                    0,
                    vec![Arc::new(0, 1)],
                    vec![Arc::new(1, 1), Arc::new(2, 1)],
                ),
                Transition::new(1, vec![Arc::new(1, 1)], vec![Arc::new(3, 1)]),
                Transition::new(2, vec![Arc::new(2, 1)], vec![Arc::new(4, 1)]),
                Transition::new(
                    3,
                    vec![Arc::new(3, 1), Arc::new(4, 1)],
                    vec![Arc::new(5, 1)],
                ),
            ],
        )?;
        let storage = Shared::new(MemoryStorage::new());
        // Demo only. Provision a random secret securely in real deployments.
        let auth = Shared::new(Authenticator::new([7; 32]));
        let p = Petri::new(definition.clone(), storage.clone(), auth.clone())?;
        let q = Petri::new(
            Definition::from_json(&definition.to_json()?)?,
            storage,
            auth,
        )?;

        let root = p.start(1001).await?;
        let branches = p.execute(1001, 0, &[root]).await?;
        let payment = Token::from_json(&branches[0].to_json()?)?;
        q.granted(1001, 1, std::slice::from_ref(&payment)).await?;
        // The payment service does its business work before recording completion.
        let paid = q.execute(1001, 1, &[payment]).await?;
        assert!(p.granted(1001, 3, &paid).await.is_err());
        println!("Paid, but packing is pending: shipping is blocked.");

        let packed = p.execute(1001, 2, &branches[1..]).await?;
        let shipped = q.execute(1001, 3, &[paid, packed].concat()).await?;
        assert_eq!(shipped[0].place(), 5);
        assert!(shipped[0].scope().is_empty());
        let snapshot = p.snapshot(1001).await?;
        assert_eq!(snapshot.tokens(), shipped.as_slice());
        assert_eq!(snapshot.revision(), 4);
        println!("Native join consumed both branch tokens. Run 1001 is at revision 4.");
        Ok(())
    })
}
