use std::io::Write as _;

#[test]
fn manifest() {
    let mut mint = goldenfile::Mint::new("tests/golden_files");
    let mut manifest_output = mint.new_goldenfile("ssbc::manifest.stdout").unwrap();
    let mut cmd = new_ssbc_cmd();
    cmd.env_remove("RUST_LOG");
    cmd.args(&["manifest"]);
    let stdout = String::from_utf8(cmd.unwrap().stdout).unwrap();
    let mut stdout_lines = stdout.split('\n').collect::<Vec<_>>();
    stdout_lines.sort_unstable();
    // We sort the output because the order depends on the ordering in the RPC
    // response which is not stable
    manifest_output
        .write_all(stdout_lines.join("\n").as_ref())
        .unwrap();
}

#[test]
fn invite_create() {
    let mut cmd = new_ssbc_cmd();
    cmd.args(&["invite", "create"]);
    cmd.unwrap();
}

#[test]
fn whoami_socket() {
    let mut cmd = new_ssbc_cmd();
    cmd.args(&["call", "whoami"]);
    cmd.unwrap();
}

fn new_ssbc_cmd() -> assert_cmd::Command {
    let mut cmd = assert_cmd::Command::cargo_bin("ssbc").unwrap();
    cmd.args(&["--socket", "/tmp/rust-ssb-test/socket"]);
    cmd
}
