use super::*;
use serde_json::json;

const UID_A: &str = "0123456789abcdef0123456789abcdef";
const UID_B: &str = "fedcba9876543210fedcba9876543210";
const T: &str = "2026-09-24T12:34:56.789Z";

fn video() -> VideoData {
    VideoData {
        uid: UID_A.into(),
        youtube_id: "dQw4w9WgXcQ".into(),
        url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into(),
        title: "Titel".into(),
        thumbnail_url: "https://i.ytimg.com/x.jpg".into(),
        thumbnail_data: Some(BASE64.encode([1u8, 2, 3])),
        transcript: Some(r#"[{"text":"hi","start":0.0}]"#.into()),
        chapters: None,
        published_at: Some("2024-01-01".into()),
        description: None,
        transcript_error: None,
        created_at: T.into(),
    }
}

#[test]
fn canonical_time_normalizes_offsets_and_precision() {
    assert_eq!(
        canonical_time("2026-09-24T12:34:56.123456789+00:00").unwrap(),
        "2026-09-24T12:34:56.123Z"
    );
    assert_eq!(
        canonical_time("2026-09-24T14:34:56+02:00").unwrap(),
        "2026-09-24T12:34:56.000Z"
    );
    assert_eq!(canonical_time(T).unwrap(), T);
    assert!(canonical_time("gestern").is_err());
    assert!(is_canonical_time(T));
    assert!(!is_canonical_time("2026-09-24T12:34:56Z"));
    assert!(is_canonical_time(&now_canonical()));
}

#[test]
fn name_key_folds_ascii_only_like_nocase() {
    assert_eq!(name_key("  KI  "), "ki");
    assert_eq!(name_key("Ärger"), "Ärger");
}

#[test]
fn ids_are_checked() {
    assert!(is_uid(UID_A));
    assert!(!is_uid("0123456789ABCDEF0123456789ABCDEF"));
    assert!(!is_uid("abc"));
    assert!(is_youtube_id("dQw4w9WgXcQ"));
    assert!(is_youtube_id("a-b_c123456"));
    assert!(!is_youtube_id("dQw4w9WgXc"));
    assert!(!is_youtube_id("dQw4w9WgXc!"));
}

#[test]
fn video_op_uses_type_tag_and_flat_camel_case_fields() {
    let op = Op::Video {
        data: video(),
        changed_at: T.into(),
    };
    let value = serde_json::to_value(&op).unwrap();
    assert_eq!(value["type"], "video");
    assert_eq!(value["youtubeId"], "dQw4w9WgXcQ");
    assert_eq!(value["changedAt"], T);
    assert!(value.get("data").is_none());
    let back: Op = serde_json::from_value(value).unwrap();
    assert_eq!(back, op);
}

#[test]
fn all_op_variants_roundtrip() {
    let ops = vec![
        Op::VideoGone {
            uid: UID_A.into(),
            reason: GoneReason::Withdrawn,
        },
        Op::Summary(SummaryData {
            uid: UID_B.into(),
            video_uid: UID_A.into(),
            created_at: T.into(),
            summary: "Text".into(),
            provider: None,
            model: Some("m".into()),
            options: None,
        }),
        Op::SummaryDelete {
            uid: UID_B.into(),
            video_uid: UID_A.into(),
        },
        Op::Chat {
            data: ChatData {
                uid: UID_B.into(),
                video_uid: UID_A.into(),
                title: "Chat".into(),
                created_at: T.into(),
                updated_at: T.into(),
                context_options: ContextOptions {
                    transcript: true,
                    summary_uids: Some(vec![]),
                },
            },
            changed_at: T.into(),
        },
        Op::ChatDelete {
            uid: UID_B.into(),
            video_uid: UID_A.into(),
        },
        Op::Round(RoundData {
            uid: UID_B.into(),
            chat_uid: UID_A.into(),
            video_uid: UID_A.into(),
            created_at: T.into(),
            messages: vec![RoundMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: Some(json!([{"id": "1"}])),
                tool_call_id: None,
                provider: None,
                model: None,
            }],
        }),
        Op::Collection {
            data: CollectionData {
                uid: UID_B.into(),
                name: "KI".into(),
                created_at: T.into(),
            },
            changed_at: T.into(),
        },
        Op::CollectionDelete { uid: UID_B.into() },
        Op::Membership {
            video_uid: UID_A.into(),
            collection_uid: UID_B.into(),
            present: false,
            changed_at: T.into(),
        },
    ];
    let text = serde_json::to_string(&PushRequest { ops: ops.clone() }).unwrap();
    let back: PushRequest = serde_json::from_str(&text).unwrap();
    assert_eq!(back.ops, ops);
    assert!(validate_all(&ops).is_ok());
    let deletes: Vec<bool> = ops.iter().map(Op::is_delete).collect();
    assert_eq!(
        deletes,
        [true, false, true, false, true, false, false, true, false]
    );
}

#[test]
fn null_and_empty_summary_uids_stay_distinct() {
    let null: ContextOptions =
        serde_json::from_value(json!({"transcript": true, "summaryUids": null})).unwrap();
    let empty: ContextOptions =
        serde_json::from_value(json!({"transcript": true, "summaryUids": []})).unwrap();
    assert_eq!(null.summary_uids, None);
    assert_eq!(empty.summary_uids, Some(vec![]));
    assert_eq!(
        serde_json::to_value(&null).unwrap()["summaryUids"],
        Value::Null
    );
}

#[test]
fn server_states_roundtrip_with_seq() {
    let states = vec![
        ServerState {
            seq: 1,
            state: State::Video(video()),
        },
        ServerState {
            seq: 2,
            state: State::VideoGone {
                uid: UID_B.into(),
                reason: GoneReason::Merged,
                merged_into: Some(UID_A.into()),
            },
        },
        ServerState {
            seq: 3,
            state: State::Membership {
                video_uid: UID_A.into(),
                collection_uid: UID_B.into(),
                present: true,
            },
        },
    ];
    let value = serde_json::to_value(PullResponse {
        states: states.clone(),
        next: 3,
        more: false,
    })
    .unwrap();
    assert_eq!(value["states"][1]["type"], "videoGone");
    assert_eq!(value["states"][1]["mergedInto"], UID_A);
    assert_eq!(value["states"][1]["seq"], 2);
    let back: PullResponse = serde_json::from_value(value).unwrap();
    assert_eq!(back.states, states);
    assert_eq!(
        states[2].state.entity_key(),
        ("membership", format!("{UID_A}/{UID_B}"))
    );
    assert_eq!(states[1].state.entity_key(), ("video", UID_B.to_string()));
}

#[test]
fn validation_rejects_structural_errors_with_index() {
    let good = Op::CollectionDelete { uid: UID_A.into() };
    let mut bad_video = video();
    bad_video.thumbnail_data = Some("%%%".into());
    let ops = vec![
        good.clone(),
        Op::Video {
            data: bad_video,
            changed_at: T.into(),
        },
    ];
    let (index, message) = validate_all(&ops).unwrap_err();
    assert_eq!(index, 1);
    assert!(message.contains("thumbnailData"));

    let cases = vec![
        Op::VideoGone {
            uid: UID_A.into(),
            reason: GoneReason::Merged,
        },
        Op::CollectionDelete { uid: "x".into() },
        Op::Membership {
            video_uid: UID_A.into(),
            collection_uid: UID_B.into(),
            present: true,
            changed_at: "2026-09-24T12:34:56Z".into(),
        },
        Op::Collection {
            data: CollectionData {
                uid: UID_A.into(),
                name: "   ".into(),
                created_at: T.into(),
            },
            changed_at: T.into(),
        },
        Op::Round(RoundData {
            uid: UID_A.into(),
            chat_uid: UID_B.into(),
            video_uid: UID_A.into(),
            created_at: T.into(),
            messages: vec![RoundMessage {
                role: "system".into(),
                content: "x".into(),
                tool_calls: None,
                tool_call_id: None,
                provider: None,
                model: None,
            }],
        }),
    ];
    for op in cases {
        assert!(op.validate().is_err(), "{op:?}");
    }

    let mut huge = video();
    huge.thumbnail_data = Some(BASE64.encode(vec![0u8; MAX_THUMBNAIL_BYTES + 1]));
    assert!(Op::Video {
        data: huge,
        changed_at: T.into()
    }
    .validate()
    .is_err());

    let mut broken_json = video();
    broken_json.transcript = Some("{kaputt".into());
    assert!(Op::Video {
        data: broken_json,
        changed_at: T.into()
    }
    .validate()
    .is_err());
}

#[test]
fn op_result_omits_empty_fields() {
    let ok = OpResult {
        status: OpStatus::Ok,
        reason: None,
        missing: None,
    };
    assert_eq!(serde_json::to_value(&ok).unwrap(), json!({"status": "ok"}));
    let retry: OpResult =
        serde_json::from_value(json!({"status": "retry", "missing": "video/abc"})).unwrap();
    assert_eq!(retry.status, OpStatus::Retry);
    assert_eq!(retry.missing.as_deref(), Some("video/abc"));
}
