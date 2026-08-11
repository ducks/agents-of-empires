use aoe_domain::{Event, EventEnvelope, MatchState, TerritoryState};
use aoe_replay::{WorldState, reduce};
use aoe_tui::{RenderOptions, render_world};

fn envelope(sequence: u64, event: Event) -> EventEnvelope {
    EventEnvelope {
        schema_version: 1,
        sequence,
        elapsed_ms: sequence * 1_000,
        event,
    }
}

fn fixture() -> (WorldState, Vec<EventEnvelope>) {
    let events = vec![
        envelope(
            0,
            Event::TerritoryRegistered {
                territory: "gatekeeper".into(),
                class: "edge".into(),
                agent: "agent-a".into(),
            },
        ),
        envelope(
            1,
            Event::MatchStateChanged {
                from: MatchState::Preparing,
                to: MatchState::Running,
            },
        ),
        envelope(
            2,
            Event::TerritoryStateChanged {
                territory: "gatekeeper".into(),
                from: TerritoryState::Provisioning,
                to: TerritoryState::Healthy,
                reason: "preflight passed".into(),
            },
        ),
        envelope(
            3,
            Event::HealthObserved {
                territory: "gatekeeper".into(),
                healthy: true,
                status: Some(200),
                latency_ms: 3,
                detail: "ok".into(),
            },
        ),
    ];
    let mut state = WorldState::default();
    for event in &events {
        reduce(&mut state, event);
    }
    (state, events)
}

#[test]
fn no_color_output_uses_text_markers() {
    let (state, events) = fixture();
    let rendered = render_world(
        &state,
        &events,
        RenderOptions {
            color: false,
            ..RenderOptions::default()
        },
    );
    assert!(rendered.contains("[+] gatekeeper"));
    assert!(rendered.contains("edge"));
    assert!(!rendered.contains("\x1b["));
}

#[test]
fn narrow_layout_remains_legible() {
    let (state, events) = fixture();
    let rendered = render_world(
        &state,
        &events,
        RenderOptions {
            width: 32,
            color: false,
            ..RenderOptions::default()
        },
    );
    assert!(rendered.contains("gatekeeper [edge]: up"));
    assert!(!rendered.contains("territory          state"));
}
