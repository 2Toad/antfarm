fn main() {
    let paths = antfarm_core::Paths::from_env();
    println!("antfarm: watching {}", paths.claude_home.display());
}
