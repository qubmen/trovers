#[cfg(test)]
mod tests {
    use crate::config::Config;

    /// A config.toml written before video playback existed. `Config::load` reads a
    /// fixed path under the user's home, so the parse is tested where the decision
    /// actually lives: serde.
    const LEGACY: &str = r#"
default_speed = 1.0
default_volume = 80
"#;

    #[test]
    fn a_config_without_video_args_loads_with_none() {
        let config: Config = toml::from_str(LEGACY).expect("legacy config parses");
        assert!(
            config.video_mpv_args.is_empty(),
            "an absent key must not invent flags: mpv exits on one it does not know"
        );
    }

    #[test]
    fn the_default_config_carries_no_video_args() {
        assert!(Config::default().video_mpv_args.is_empty());
    }

    #[test]
    fn configured_video_args_round_trip() {
        let config = Config {
            video_mpv_args: vec!["--focus-on=never".to_string(), "--ontop".to_string()],
            ..Config::default()
        };
        let raw = toml::to_string(&config).expect("serializes");
        let back: Config = toml::from_str(&raw).expect("parses back");
        assert_eq!(
            back.video_mpv_args,
            vec!["--focus-on=never".to_string(), "--ontop".to_string()]
        );
    }
}
