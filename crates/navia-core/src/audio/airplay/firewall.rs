//! Opens the ports AirPlay receivers connect back on, in a Linux firewall.
//! Supports ufw and firewalld through `pkexec`, and opens only these ports to the receiver's network.

use std::net::Ipv4Addr;
use std::process::Command;

use super::session::Ports;

/// A firewall Navia knows how to add a rule to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirewallKind {
    Ufw,
    Firewalld,
}

impl FirewallKind {
    pub fn name(self) -> &'static str {
        match self {
            FirewallKind::Ufw => "ufw",
            FirewallKind::Firewalld => "firewalld",
        }
    }
}

/// The rule's commands, each as its argument list (run as root).
pub fn rule_commands(kind: FirewallKind, net: &str, ports: &Ports) -> Vec<Vec<String>> {
    let udp = format!("{}:{}", ports.timing.min(ports.data), ports.timing.max(ports.data));
    let udp_range = udp.replace(':', "-");
    let tcp = ports.dacp.to_string();
    let v = |a: &[&str]| a.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    match kind {
        FirewallKind::Ufw => vec![
            v(&["ufw", "allow", "proto", "udp", "from", net, "to", "any", "port", &udp, "comment", "Navia AirPlay"]),
            v(&["ufw", "allow", "proto", "tcp", "from", net, "to", "any", "port", &tcp, "comment", "Navia AirPlay"]),
        ],
        FirewallKind::Firewalld => vec![
            v(&["firewall-cmd", "--permanent", &format!("--add-rich-rule=rule family=ipv4 source address={net} port port={udp_range} protocol=udp accept")]),
            v(&["firewall-cmd", "--permanent", &format!("--add-rich-rule=rule family=ipv4 source address={net} port port={tcp} protocol=tcp accept")]),
            v(&["firewall-cmd", "--reload"]),
        ],
    }
}

/// The commands as someone would type them, for the prompt's disclosure.
pub fn rule_text(kind: FirewallKind, net: &str, ports: &Ports) -> String {
    rule_commands(kind, net, ports)
        .iter()
        .map(|argv| {
            let words: Vec<String> = argv.iter().map(|w| quote(w)).collect();
            format!("sudo {}", words.join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Single-quotes a word for a POSIX shell when it needs it.
fn quote(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=,".contains(c)) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// The receiver's network as `a.b.c.d/len`, from `ip -o -4 addr` output.
/// Falls back to the receiver alone (`/32`) when no interface's network holds it.
pub fn network_for(receiver: Ipv4Addr, ip_addr_output: &str) -> String {
    for line in ip_addr_output.lines() {
        let mut words = line.split_whitespace();
        while let Some(w) = words.next() {
            if w != "inet" {
                continue;
            }
            let Some((addr, len)) = words.next().and_then(|c| c.split_once('/')) else { break };
            let (Ok(addr), Ok(len)) = (addr.parse::<Ipv4Addr>(), len.parse::<u32>()) else { break };
            if len == 0 || len > 32 {
                break;
            }
            let mask = u32::MAX << (32 - len);
            if u32::from(addr) & mask == u32::from(receiver) & mask {
                return format!("{}/{len}", Ipv4Addr::from(u32::from(addr) & mask));
            }
        }
    }
    format!("{receiver}/32")
}

/// The receiver's local network, read from this computer's interfaces.
pub fn local_network(receiver: Ipv4Addr) -> String {
    let out = Command::new("ip").args(["-o", "-4", "addr", "show"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    network_for(receiver, &out)
}

/// Which known firewall is running, checked without root.
pub fn detect() -> Option<FirewallKind> {
    let says = |cmd: &str, args: &[&str], want: &str| Command::new(cmd).args(args).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == want);
    if says("firewall-cmd", &["--state"], "running") {
        return Some(FirewallKind::Firewalld);
    }
    if says("systemctl", &["is-active", "ufw"], "active") {
        return Some(FirewallKind::Ufw);
    }
    None
}

/// Adds the rule through the system password prompt. Blocks, so call it off the UI thread.
pub fn allow(kind: FirewallKind, net: &str, ports: &Ports) -> Result<(), String> {
    // One prompt for all commands: `sh -c` with every word quoted.
    let script = rule_commands(kind, net, ports).iter().map(|argv| argv.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(" && ");
    let status = Command::new("pkexec").args(["/bin/sh", "-c", &script]).status().map_err(|e| format!("pkexec: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("pkexec exited with {status}")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::airplay::DEFAULT_PORTS;

    const IP: &str = "1: lo    inet 127.0.0.1/8 scope host lo\\       valid_lft forever\n3: wlan0    inet 192.168.1.23/24 brd 192.168.1.255 scope global dynamic wlan0\\       valid_lft 80000sec\n";

    #[test]
    fn network_from_interfaces() {
        assert_eq!(network_for("192.168.1.40".parse().unwrap(), IP), "192.168.1.0/24");
        assert_eq!(network_for("10.0.0.5".parse().unwrap(), IP), "10.0.0.5/32");
    }

    #[test]
    fn ufw_rule_is_only_our_ports() {
        let text = rule_text(FirewallKind::Ufw, "192.168.1.0/24", &DEFAULT_PORTS);
        assert_eq!(text, "sudo ufw allow proto udp from 192.168.1.0/24 to any port 51310:51312 comment 'Navia AirPlay'\nsudo ufw allow proto tcp from 192.168.1.0/24 to any port 51313 comment 'Navia AirPlay'");
    }

    #[test]
    fn firewalld_rule_is_only_our_ports() {
        let text = rule_text(FirewallKind::Firewalld, "192.168.1.0/24", &DEFAULT_PORTS);
        assert!(text.contains("'--add-rich-rule=rule family=ipv4 source address=192.168.1.0/24 port port=51310-51312 protocol=udp accept'"));
        assert!(text.contains("port port=51313 protocol=tcp accept"));
        assert!(text.ends_with("sudo firewall-cmd --reload"));
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote("51310:51312"), "51310:51312");
    }
}
