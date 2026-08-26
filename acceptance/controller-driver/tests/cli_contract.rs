// conformance: the physical-controller driver refuses an incomplete invocation before dispatch.

use std::process::Command;

#[test]
fn incomplete_invocation_fails_before_controller_setup() {
    let output = Command::new(env!("CARGO_BIN_EXE_sim-estate-acceptance"))
        .output()
        .expect("acceptance driver must start");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .expect("driver errors are UTF-8")
            .contains("--source is required")
    );
}
