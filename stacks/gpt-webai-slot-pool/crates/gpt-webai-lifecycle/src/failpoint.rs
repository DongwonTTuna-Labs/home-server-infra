use std::io::Write;

pub const ENV_NAME: &str = "GPT_WEBAI_FAILPOINT";

pub const NAMES: [&str; 18] = [
    "after-immutable-temp-write",
    "after-immutable-promote-before-directory-fsync",
    "after-event-append-before-head",
    "after-head-before-projection-publish",
    "after-uploadcleared",
    "after-sendclickarmed",
    "after-physical-send-click-before-provider-stdout",
    "after-turnstartconfirmed",
    "after-session-claim-lease-owner-renewal",
    "after-answerterminal",
    "after-artifact-listener-arm",
    "after-artifact-click",
    "after-playwright-host-save-before-receipt",
    "after-receipt-before-event",
    "after-terminalpersisted",
    "after-evidence-preservation",
    "after-runtime-stop-before-resource-release",
    "after-each-exactly-once-release-event",
];

pub fn validate_requested() -> Result<(), String> {
    let Some(name) = requested() else {
        return Ok(());
    };
    if is_known(&name) {
        Ok(())
    } else {
        Err(format!("unrecognised {ENV_NAME}: {name}"))
    }
}

pub fn is_known(name: &str) -> bool {
    NAMES.contains(&name)
}

pub fn requested() -> Option<String> {
    std::env::var(ENV_NAME).ok().filter(|name| !name.is_empty())
}

pub fn hit(name: &'static str) {
    debug_assert!(is_known(name));
    if requested().as_deref() == Some(name) {
        exit_now(name);
    }
}

pub fn propagate_provider_exit(code: i32, stdout: &[u8], stderr: &[u8]) {
    if code != 99 || !stdout.is_empty() {
        return;
    }
    let Some(name) = requested() else {
        return;
    };
    let expected = format!("failpoint:{name}\n");
    if is_known(&name) && stderr == expected.as_bytes() {
        exit_now(&name);
    }
}

fn exit_now(name: &str) -> ! {
    let line = format!("failpoint:{name}\n");
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(line.as_bytes());
    let _ = stderr.flush();
    std::process::exit(99)
}

#[cfg(test)]
mod tests {
    use super::{is_known, NAMES};

    #[test]
    fn canonical_failpoint_set_is_closed_and_unique() {
        assert_eq!(NAMES.len(), 18);
        let unique = NAMES.into_iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), 18);
        assert!(NAMES.into_iter().all(is_known));
        assert!(!is_known("after-unknown-effect"));
    }
}
