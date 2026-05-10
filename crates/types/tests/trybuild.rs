#![allow(missing_docs)]

#[test]
fn literal_macros_reject_invalid_inputs_at_compile_time() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/bad_container_id.rs");
    t.compile_fail("tests/ui/bad_virtiofs_tag.rs");
    t.compile_fail("tests/ui/bad_hostname.rs");
}
