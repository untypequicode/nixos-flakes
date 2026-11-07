//! Notifications de bureau.

use crate::proc;
use std::process::Stdio;

/// Arguments passés à `notify-send`.
fn args<'a>(title: &'a str, body: &'a str, critical: bool) -> [&'a str; 7] {
    let urgency = if critical { "critical" } else { "normal" };
    ["-a", "nixos-flakes", "-u", urgency, "--", title, body]
}

/// Envoie une notification avec `notify-send` ; les erreurs sont ignorées.
pub fn desktop(title: &str, body: &str, critical: bool) {
    let _ = proc::command("notify-send")
        .args(args(title, body, critical))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::args;

    #[test]
    fn urgency_follows_flag() {
        assert_eq!(args("t", "b", true)[3], "critical");
        assert_eq!(args("t", "b", false)[3], "normal");
    }

    #[test]
    fn title_and_body_come_after_separator() {
        let a = args("-titre", "corps", false);
        assert_eq!(&a[4..], ["--", "-titre", "corps"]);
    }
}
