//! Offline prerequisite for the rectangular-host repair. It never opens the
//! viewer, a profile, an account, a grid connection or the production bridge.
//! Every process modifies only windows it creates. See the companion runbook.

#[cfg(windows)]
#[path = "native_frame/proof.rs"]
mod proof;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        proof::run()?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("the native-frame proof requires a local Windows desktop".into())
    }
}
