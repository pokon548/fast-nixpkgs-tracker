pub const URL: &str = "https://github.com/NixOS/nixpkgs";
pub const REPO_PATH: &str = "nixpkgs";
pub const CACHED_BRANCHES: [&str; 7] = [
    "master",
    "staging",
    "staging-next",
    "nixpkgs-unstable",
    "nixos-unstable-small",
    "nixos-unstable",
    "nixos-24.05",
];

pub enum IndexState {
    Starting,
    CloningGitRepo,
    IndexingCommit,
    Ready,
}
