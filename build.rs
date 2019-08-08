extern crate pkg_config;

fn main() {
    pkg_config::Config::new()
        .atleast_version("5.3")
        .probe("lua")
        .unwrap();
}
