use crate::errors::LifecycleError;

pub(super) fn option_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .filter(|value| !value.starts_with("--"))
        .cloned()
}

pub(super) fn optional_u64(args: &[String], name: &str, fallback: u64) -> u64 {
    option_value(args, name)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

pub(super) fn reject_unknown_options(args: &[String]) -> Result<(), LifecycleError> {
    let allowed = [
        "--json",
        "--dry-run",
        "--request-id",
        "--run-id",
        "--fencing-token",
        "--ttl-ms",
        "--docker-bin",
        "--runtime-start-timeout-ms",
    ];
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg.starts_with("--") && !allowed.contains(&arg.as_str()) {
            return Err(LifecycleError::Usage(format!("unknown option: {arg}")));
        }
        index += if matches!(
            arg.as_str(),
            "--request-id"
                | "--run-id"
                | "--fencing-token"
                | "--ttl-ms"
                | "--docker-bin"
                | "--runtime-start-timeout-ms"
        ) {
            2
        } else {
            1
        };
    }
    Ok(())
}
