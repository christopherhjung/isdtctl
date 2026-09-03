//! Compiles the snippet the repository README shows, so the two cannot drift.
use std::time::Duration;

use api::{tokens, BatteryKind, Client, LinkType, TaskType};

#[allow(dead_code)]
async fn from_the_readme() -> Result<(), Box<dyn std::error::Error>> {
    let client_id = tokens::parse("9102782c5bfb5047a4533d071feb6eca")?;
    let mut charger =
        Client::discover_bound(None, Duration::from_secs(10), Default::default(), client_id)
            .await?;

    let state = charger.work_state(0).await?;
    println!("{} at {}%", state.state.label(), state.capacity_percent);

    charger
        .start_task(
            0,
            TaskType::Charge,
            BatteryKind::LiPo,
            LinkType::SerialOnly,
            2000,
            4,
            4200,
        )
        .await?;
    Ok(())
}

fn main() {
    println!("the README snippet compiles");
}
