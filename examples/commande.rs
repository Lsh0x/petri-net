//! Business data belongs to the application, not to the Petri tokens.
use petri_core::engine::{Authenticator, MemoryStorage};
use petri_core::workflow::{Petri, TaskToken, Workflow};
use std::sync::Arc;

type Moteur = Petri<MemoryStorage>;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Commande {
    numero: u64,
    montant_centimes: u64,
    payee: bool,
    preparee: bool,
    expediee: bool,
}

async fn payer(p: &Moteur, token: TaskToken, commande: &mut Commande) -> Result<()> {
    p.granted_for(1, &token).await?;
    // Business code: in production, call your payment provider here.
    if commande.montant_centimes == 0 {
        return Err("le montant doit être positif".into());
    }
    println!("Paiement de {} centimes accepté", commande.montant_centimes);
    commande.payee = true;
    p.executed(token).await?;
    Ok(())
}

async fn preparer(p: &Moteur, token: TaskToken, commande: &mut Commande) -> Result<()> {
    p.granted_for(2, &token).await?;
    // Business code: reserve products and prepare the parcel.
    println!("Colis de la commande {} préparé", commande.numero);
    commande.preparee = true;
    p.executed(token).await?;
    Ok(())
}

async fn expedier(p: &Moteur, token: TaskToken, commande: &mut Commande) -> Result<()> {
    p.granted_for(3, &token).await?;
    // Business code: hand the parcel to the carrier.
    println!("Commande {} expédiée", commande.numero);
    commande.expediee = true;
    p.executed(token).await?;
    Ok(())
}

async fn traiter(p: &Moteur, run: u128, commande: &mut Commande) -> Result<()> {
    p.start(run).await?; // Execution ID only. No business payload.
    while !p.status(run).await?.finished {
        for token in p.ready(run).await? {
            match token.task_id() {
                1 => payer(p, token, commande).await?,
                2 => preparer(p, token, commande).await?,
                3 => expedier(p, token, commande).await?,
                _ => unreachable!("validated workflow contains three known tasks"),
            }
        }
    }
    Ok(())
}

fn moteur() -> Result<Moteur> {
    Ok(Petri::new(
        Workflow::from_yaml(include_str!("workflows/commande.yaml"))?,
        Arc::new(MemoryStorage::new()),
        Arc::new(Authenticator::new([7; 32])), // Demo key only.
    )?)
}

fn main() -> Result<()> {
    futures::executor::block_on(async {
        let p = moteur()?;
        // Application data, never passed to the Petri constructor or tokens.
        let mut commande = Commande {
            numero: 1001,
            montant_centimes: 2500,
            payee: false,
            preparee: false,
            expediee: false,
        };
        traiter(&p, 42, &mut commande).await?;
        assert!(commande.payee && commande.preparee && commande.expediee);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn commande(montant_centimes: u64) -> Commande {
        Commande {
            numero: 1001,
            montant_centimes,
            payee: false,
            preparee: false,
            expediee: false,
        }
    }
    #[test]
    fn business_failure_does_not_complete_payment_or_unlock_shipping() {
        futures::executor::block_on(async {
            let p = moteur().unwrap();
            let mut c = commande(0);
            let mut roots = p.start(42).await.unwrap();
            assert!(payer(&p, roots.remove(0), &mut c).await.is_err());
            assert!(!c.payee);
            assert!(p.status(42).await.unwrap().completed.is_empty());
            assert_eq!(
                p.ready(42)
                    .await
                    .unwrap()
                    .iter()
                    .map(TaskToken::task_id)
                    .collect::<Vec<_>>(),
                vec![1, 2]
            );
        });
    }
    #[test]
    fn wrong_function_rejects_token_before_business_work() {
        futures::executor::block_on(async {
            let p = moteur().unwrap();
            let mut c = commande(2500);
            let preparation = p.start(42).await.unwrap().remove(1);
            assert!(payer(&p, preparation, &mut c).await.is_err());
            assert!(!c.payee);
            assert!(p.status(42).await.unwrap().completed.is_empty());
        });
    }
    #[test]
    fn both_completion_orders_unlock_shipping() {
        futures::executor::block_on(async {
            for preparation_first in [false, true] {
                let p = moteur().unwrap();
                let mut c = commande(2500);
                let mut roots = p.start(42).await.unwrap();
                let payment = roots.remove(0);
                let preparation = roots.remove(0);
                if preparation_first {
                    preparer(&p, preparation, &mut c).await.unwrap();
                    assert!(p.ready(42).await.unwrap().iter().all(|t| t.task_id() != 3));
                    payer(&p, payment, &mut c).await.unwrap();
                } else {
                    payer(&p, payment, &mut c).await.unwrap();
                    assert!(p.ready(42).await.unwrap().iter().all(|t| t.task_id() != 3));
                    preparer(&p, preparation, &mut c).await.unwrap();
                }
                expedier(&p, p.ready(42).await.unwrap().remove(0), &mut c)
                    .await
                    .unwrap();
                assert!(c.payee && c.preparee && c.expediee);
                assert!(p.status(42).await.unwrap().finished);
            }
        });
    }
}
