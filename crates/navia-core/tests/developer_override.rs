//! The developer override, where `NAVIDROME_*` only count under `NAVIA_DEV=1`.
//! One test, because it changes this process's environment.

use navia_core::connect::{developer_login, developer_override};

#[test]
fn navidrome_variables_count_only_under_navia_dev() {
    // SAFETY: This is the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe {
        std::env::remove_var("NAVIA_DEV");
        std::env::set_var("NAVIDROME_URL", "http://127.0.0.1:9");
        std::env::set_var("NAVIDROME_USER", "dev");
        std::env::set_var("NAVIDROME_PASSWORD", "x");
    }
    assert!(!developer_override());
    assert_eq!(developer_login(), None, "a user's run ignores the variables");
    for off in ["0", "", "true", "yes"] {
        unsafe { std::env::set_var("NAVIA_DEV", off) };
        assert_eq!(developer_login(), None, "NAVIA_DEV={off:?} isn't on");
    }
    unsafe { std::env::set_var("NAVIA_DEV", "1") };
    assert!(developer_override());
    assert_eq!(developer_login(), Some(("http://127.0.0.1:9".into(), "dev".into(), "x".into())));
    unsafe { std::env::set_var("NAVIDROME_PASSWORD", "") };
    assert_eq!(developer_login(), None, "all three are needed");
}
