//! Which package a file of an analysed tree belongs to.

/// The install a root-relative `/`-separated path belongs to and the path
/// inside it. The innermost `node_modules/<name>` (or
/// `node_modules/@scope/<name>`) is the install, named by its directory from
/// the root, so two installs of one package are two packages. A path outside
/// every `node_modules` belongs to the root package, named `.`.
pub(crate) fn install_of(path: &str) -> (&str, &str) {
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
    let end = start
        + rest
            .match_indices('/')
            .nth(segments - 1)
            .map_or(rest.len(), |(at, _)| at);
    let inner = path.get(end + 1..).unwrap_or("");
    (&path[..end], inner)
}

#[cfg(test)]
mod tests {
    use super::install_of;

    #[test]
    fn a_file_belongs_to_its_innermost_node_modules_install() {
        assert_eq!(install_of("main.js"), (".", "main.js"));
        assert_eq!(
            install_of("node_modules/liba/index.js"),
            ("node_modules/liba", "index.js")
        );
        assert_eq!(
            install_of("node_modules/@scope/pkg/lib/a.js"),
            ("node_modules/@scope/pkg", "lib/a.js")
        );
        assert_eq!(
            install_of("node_modules/a/node_modules/b/x.js"),
            ("node_modules/a/node_modules/b", "x.js")
        );
        assert_eq!(
            install_of("src/node_modules_x/a.js"),
            (".", "src/node_modules_x/a.js")
        );
        assert_eq!(install_of("node_modules/a"), ("node_modules/a", ""));
    }

    #[test]
    fn two_installs_of_one_package_are_two_packages() {
        assert_ne!(
            install_of("node_modules/dual/index.cjs").0,
            install_of("node_modules/outer/node_modules/dual/index.cjs").0
        );
    }
}
