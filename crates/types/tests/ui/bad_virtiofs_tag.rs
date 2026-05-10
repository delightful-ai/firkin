use firkin_types::virtiofs_tag;

fn main() {
    let _ = virtiofs_tag!("this-tag-is-longer-than-thirty-six-bytes");
}
