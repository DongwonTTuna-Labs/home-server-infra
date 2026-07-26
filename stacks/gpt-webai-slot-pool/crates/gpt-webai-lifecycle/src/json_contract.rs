use std::io::Write;

use serde::Serialize;

pub fn print_json<T: Serialize>(value: &T) -> Result<(), crate::errors::LifecycleError> {
    let canonical = serde_json::to_value(value)?;
    let mut bytes = serde_json::to_vec(&canonical)?;
    bytes.push(b'\n');
    let stdout = std::io::stdout();
    let mut locked = stdout.lock();
    locked.write_all(&bytes)?;
    locked.flush()?;
    Ok(())
}
