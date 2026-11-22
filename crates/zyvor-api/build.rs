// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

// Rebuild when migrations change, so embedded migrations never go stale.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
