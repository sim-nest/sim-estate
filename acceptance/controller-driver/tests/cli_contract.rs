// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: the physical-controller driver refuses an incomplete invocation before dispatch.

use std::process::Command;

#[test]
fn incomplete_invocation_fails_before_controller_setup() {
    let output = Command::new(env!("CARGO_BIN_EXE_sim-estate-acceptance"))
        .output()
        .expect("acceptance driver must start");

    assert!(!output.status.success());
    assert_eq!(output.stdout, b"");
    assert!(
        String::from_utf8(output.stderr)
            .expect("driver errors are UTF-8")
            .contains("--source is required")
    );
}
