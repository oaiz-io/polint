//! Which package a file of an analysed tree belongs to.

/// The package a root-relative `/`-separated path belongs to and the path
/// inside it: the innermost `node_modules/<name>` (or
/// `node_modules/@scope/<name>`) names the package, and a path outside every
/// `node_modules` belongs to the root package, named `.`.
pub(crate) fn package_of(path: &str) -> (&str, &str) {
    let mut name_start = None;
    let mut offset = 0;
    for part in path.split('/') {
        if part == "node_modules" {
            name_start = Some(offset + part.len() + 1);
        }
        offset += part.len() + 1;
    }
    let Some(start) = name_start.filter(|start| *start < path.len()) else {
        return (".", path);
    };
    let rest = &path[start..];
    let segments = if rest.starts_with('@') { 2 } else { 1 };
    let end = rest
        .match_indices('/')
        .nth(segments - 1)
        .map_or(rest.len(), |(at, _)| at);
    let inner = rest.get(end + 1..).unwrap_or("");
    (&rest[..end], inner)
}

#[cfg(test)]
mod tests {
    use super::package_of;

    #[test]
    fn a_file_belongs_to_its_innermost_node_modules_package() {
        assert_eq!(package_of("main.js"), (".", "main.js"));
        assert_eq!(
            package_of("node_modules/liba/index.js"),
            ("liba", "index.js")
        );
        assert_eq!(
            package_of("node_modules/@scope/pkg/lib/a.js"),
            ("@scope/pkg", "lib/a.js")
        );
        assert_eq!(
            package_of("node_modules/a/node_modules/b/x.js"),
            ("b", "x.js")
        );
        assert_eq!(
            package_of("src/node_modules_x/a.js"),
            (".", "src/node_modules_x/a.js")
        );
    }
}
