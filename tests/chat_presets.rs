mod tests {
    use chk::presets::ChatPresets;
    use ramp::prism::air::Name;

    fn contact_names() -> (Name, Name) {
        (
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
                .parse()
                .unwrap(),
            "0379be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
                .parse()
                .unwrap(),
        )
    }

    #[test]
    fn selection_keys_round_trip_and_deduplicate_in_selection_order() {
        let (first, second) = contact_names();
        assert_eq!(
            ChatPresets::selected_recipients(&[second.to_string(), first.to_string(), second.to_string()]),
            Ok(vec![second, first])
        );
    }

    #[test]
    fn invalid_or_empty_selection_never_becomes_a_partial_recipient_list() {
        let (first, _) = contact_names();
        assert!(ChatPresets::selected_recipients(&[]).is_err());
        assert!(ChatPresets::selected_recipients(&[first.to_string(), "invalid contact".into()]).is_err());
        assert!(ChatPresets::selected_recipients(&[String::new(), first.to_string()]).is_err());
    }
}
