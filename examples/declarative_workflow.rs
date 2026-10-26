//! Run any of the three dependency graphs without defining internal Petri arcs.
use petri_core::engine::{Authenticator, MemoryStorage};
use petri_core::workflow::{Petri, Workflow};
use serde_json::json;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    futures::executor::block_on(async {
        for (name, yaml) in [
            ("order", include_str!("workflows/order.yaml")),
            ("media", include_str!("workflows/media.yaml")),
            ("deploy", include_str!("workflows/deploy.yaml")),
        ] {
            let workflow = Workflow::from_yaml(yaml)?;
            let p = Petri::new(
                workflow,
                Arc::new(MemoryStorage::new()),
                Arc::new(Authenticator::new([7; 32])),
            )?; // Demo key only.
            p.start(1001, json!({"request": 1001})).await?;
            loop {
                let status = p.status(1001).await?;
                if status.finished {
                    break;
                }
                assert!(!status.ready.is_empty());
                println!(
                    "{name}: ready {:?}",
                    status.ready.iter().map(|t| t.task_id()).collect::<Vec<_>>()
                );
                // These tasks can be dispatched concurrently to workers. This
                // example records them sequentially to keep the output readable.
                for token in status.ready {
                    p.granted(&token).await?;
                    let result = json!({"completed_by": token.task_id()});
                    // Your business work belongs here, before recording success.
                    p.executed(token, result).await?;
                }
            }
            println!("{name}: finished");
        }
        Ok(())
    })
}
