#![allow(missing_docs)]

use sideromelane_core::project_name;

#[test]
fn project_exports_name() {
    assert_eq!(project_name(), "Sideromelane");
}
