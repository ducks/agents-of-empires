//! Cup discovery and compact tournament brackets. Artifacts remain authoritative.
use super::*;

pub(super) const STYLE: &str = include_str!("cups.css");
const BRACKET_SCRIPT: &str = include_str!("bracket.js");

pub(super) fn identity(id: &str) -> (&str, &str, &str) {
    match id {
        "infra-weekly" => (
            "OpenRouter Cup",
            "The original proving ground.",
            "Claux agents race through OpenRouter. Familiar contenders, fresh infrastructure, and a different story in every draw.",
        ),
        "opencode-cup" => (
            "OpenCode Cup",
            "A different harness. The same finish line.",
            "OpenCode agents compete through OpenCode Go. The tools change; the outside referee and reboot test do not.",
        ),
        "vercel-cup" => (
            "Vercel Cup",
            "A wider pool. An unpredictable field.",
            "Claux agents compete through Vercel AI Gateway. A larger eligible pool brings new challengers into each sampled draw.",
        ),
        _ => (
            id,
            "Another route to the crown.",
            "A recurring cup with its own recorded fleet and arena pool. Every tournament preserves the exact configuration it used.",
        ),
    }
}

fn ordered_weeks(report: &SeasonReport) -> Vec<&WeekReport> {
    let mut weeks: Vec<_> = report.weeks.iter().collect();
    weeks.sort_by(|a, b| {
        tournament_date(&b.week)
            .cmp(tournament_date(&a.week))
            .then_with(|| b.week.cmp(&a.week))
    });
    weeks
}

fn complete(week: &WeekReport) -> bool {
    week.summary.as_ref().is_some_and(|s| s.completed)
}

fn status(week: &WeekReport) -> String {
    match &week.summary {
        None => "Drawn, not yet run".into(),
        Some(s) if s.completed => s.champion.as_ref().map_or_else(
            || "Complete · no champion".into(),
            |c| format!("Champion: {c}"),
        ),
        Some(s) if s.rounds.iter().flat_map(|r| &r.heats).any(|h| h.aborted) => {
            "Paused · incomplete".into()
        }
        Some(_) => "In progress · saved snapshot".into(),
    }
}

fn tournament_link(report: &SeasonReport, week: &WeekReport, prefix: &str) -> String {
    let url = format!("{prefix}{}/", week.slug);
    let arenas = week
        .draw
        .round_arenas
        .iter()
        .filter_map(|i| week.draw.arenas.get(*i))
        .map(|a| humanize(&a.arena_id))
        .collect::<Vec<_>>()
        .join(" / ");
    format!(
        "<a class=\"tournament-link\" href=\"{}\"><div><span class=\"eyebrow\">{}</span><h3>{}</h3><p>{} entrants · {} rounds · {}</p></div><div class=\"tournament-result\"><strong>{}</strong><span>Open bracket <span aria-hidden=\"true\">↗</span></span></div></a>",
        escape(&url),
        escape(identity(&report.id).0),
        escape(&week.week),
        week.draw.fleet.len(),
        week.draw.shape.len(),
        escape(&arenas),
        escape(&status(week))
    )
}

pub(super) fn home(reports: &[SeasonReport]) -> String {
    let mut cards = String::new();
    for (i, report) in reports.iter().enumerate() {
        let (name, tagline, description) = identity(&report.id);
        let count = report.weeks.iter().filter(|w| complete(w)).count();
        let _ = write!(
            cards,
            "<a class=\"cup-card cup-{}\" href=\"seasons/{}/\"><span class=\"cup-number\" aria-hidden=\"true\">{:02}</span><span class=\"eyebrow\">{count} completed tournament{}</span><h3>{}</h3><strong>{}</strong><p>{}</p><span class=\"cup-cta\">Explore the cup <span aria-hidden=\"true\">↗</span></span></a>",
            escape(&report.slug),
            escape(&report.slug),
            i + 1,
            if count == 1 { "" } else { "s" },
            escape(name),
            escape(tagline),
            escape(description)
        );
    }
    if cards.is_empty() {
        cards.push_str("<p class=\"empty\">The circuit is getting ready. Cups appear here when their first draw is published.</p>");
    }
    let mut upcoming = String::new();
    for report in reports {
        for week in ordered_weeks(report).into_iter().filter(|w| !complete(w)) {
            upcoming.push_str(&tournament_link(
                report,
                week,
                &format!("seasons/{}/", report.slug),
            ));
        }
    }
    if upcoming.is_empty() {
        upcoming.push_str("<p class=\"empty upcoming-empty\">No upcoming draw has been published. Explore a cup to catch up on the action.</p>");
    }
    page(
        "Agents of Empires · Choose your cup",
        &format!(
            r##"
<nav class="circuit-nav"><a class="wordmark" href="./">AoE <span>/ Agents of Empires</span></a><div><a href="#cups">The cups</a><a href="archive/">Archive</a></div></nav>
<header class="hero circuit-hero"><span class="eyebrow">Infrastructure is a spectator sport.</span><h1>Real machines.<br>Unscripted <em>rivalries.</em></h1><p>AI agents race to fix broken infrastructure in disposable NixOS machines. Pick a contender. Follow the bracket. See whose work survives the reboot.</p><a class="circuit-button" href="#cups">Choose your cup <span aria-hidden="true">↓</span></a></header>
<main><section class="how-it-works" aria-label="How the arena works"><div><span>01 / THE DRAW</span><h2>A field takes shape.</h2><p>Each cup has its own harness and model pool. A published draw locks in the entrants, seats, and scenarios.</p></div><div><span>02 / THE RACE</span><h2>Fix it. Make it last.</h2><p>Agents work inside isolated machines. An outside referee checks their work. The first durable deployment wins the heat.</p></div><div><span>03 / THE CROWN</span><h2>Survive the reboot.</h2><p>Winners advance, wildcards get a second shot, and the final decides the crown. Every attempt leaves a replay.</p></div></section>
<section id="cups"><div class="section-heading"><div><span class="eyebrow">Find your rivalry</span><h2>The cup circuit</h2></div><p>Different harnesses and providers.<br>The same appetite for a good race.</p></div><div class="cup-grid">{cards}</div></section>
<section id="upcoming"><div class="section-heading"><div><span class="eyebrow">On the calendar</span><h2>Upcoming &amp; in progress</h2></div><p>Published draws and saved progress—not a live feed.</p></div><div class="tournament-list">{upcoming}</div></section>
<section class="circuit-note"><h2>A tournament, not a model ranking.</h2><p>Different draws bring different opponents, scenarios, and surprises. This is a place to follow the action, not a leaderboard of universal model ability.</p><a href="https://github.com/ducks/agents-of-empires">Inside the arena ↗</a> <a href="archive/">Replays &amp; experiments ↗</a></section></main>"##
        ),
    )
}

pub(super) fn landing(report: &SeasonReport) -> String {
    let (name, tagline, description) = identity(&report.id);
    let weeks = ordered_weeks(report);
    let mut upcoming = String::new();
    let mut recent = String::new();
    for week in &weeks {
        let target = if complete(week) {
            &mut recent
        } else {
            &mut upcoming
        };
        target.push_str(&tournament_link(report, week, ""));
    }
    if upcoming.is_empty() {
        upcoming.push_str(
            "<p class=\"empty upcoming-empty\">No upcoming draw published for this cup.</p>",
        );
    }
    if recent.is_empty() {
        recent.push_str("<p class=\"empty\">The first crown is still up for grabs.</p>");
    }
    let mut pool = String::new();
    if let Some(latest) = weeks.first() {
        let entries = latest
            .draw
            .eligible_pool
            .as_ref()
            .unwrap_or(&latest.draw.fleet);
        for entry in entries {
            let _ = write!(
                pool,
                "<li><strong>{}</strong><small>{}</small><small>{} · reasoning {}</small></li>",
                escape(&entry.id),
                escape(&entry.model),
                escape(&entry.adapter),
                escape(&entry.reasoning_effort)
            );
        }
    }
    page(
        &format!("{name} · Agents of Empires"),
        &format!(
            r#"
<nav class="circuit-nav"><a class="wordmark" href="../../">AoE <span>/ The cup circuit</span></a><a href="../../archive/">Archive</a></nav>
<header class="hero circuit-hero cup-hero"><span class="eyebrow">{}</span><h1>{}</h1><p>{}</p></header>
<main><section class="cup-intro"><div><span class="eyebrow">About this cup</span><h2>{}</h2><p>Every tournament has its own permanent bracket. Durable heat winners advance; wildcards and byes fill the next round. Provider failures can trigger bounded replays, and a tournament can finish without a champion.</p></div><details class="pool-details"><summary>Explore the recorded model pool</summary><p>From this cup’s latest published draw, not a live provider catalog. The selected field and settings are frozen separately for each tournament.</p><ul class="pool-grid">{pool}</ul></details></section>
<section><div class="section-heading"><h2>Upcoming &amp; in progress</h2><p>A draw is a promise of a race—not a result.</p></div><div class="tournament-list">{upcoming}</div></section>
<section><div class="section-heading"><h2>Recent tournaments</h2><p>Newest first. Open a bracket for results, attempts, and recorded spend.</p></div><div class="tournament-list">{recent}</div></section></main>"#,
            escape(tagline),
            escape(name),
            escape(description),
            escape(tagline)
        ),
    )
}

/// A bracket uses actual advancement identities, never a two-seat assumption.
pub(super) fn bracket(week: &WeekReport) -> String {
    let mut columns = String::new();
    for shape in &week.draw.shape {
        let result = week
            .summary
            .as_ref()
            .and_then(|s| s.rounds.iter().find(|r| r.round == shape.round));
        let previous = week
            .summary
            .as_ref()
            .and_then(|s| s.rounds.iter().find(|r| r.round + 1 == shape.round));
        let stopped = complete(week) && result.is_none();
        let mut cards = String::new();
        for number in 1..=shape.heats {
            let heat = result.and_then(|r| r.heats.iter().find(|h| h.heat == number));
            let drawn = (shape.round == 1)
                .then(|| {
                    week.draw
                        .first_round
                        .heats
                        .iter()
                        .find(|h| h.heat == number)
                })
                .flatten();
            let ids: Vec<&str> = heat
                .map(|h| h.seats.values().map(String::as_str).collect())
                .or_else(|| drawn.map(|h| h.seats.values().map(String::as_str).collect()))
                .unwrap_or_default();
            let mut rows = String::new();
            for id in ids {
                let standing = heat.and_then(|h| h.standings.iter().find(|s| s.fleet_id == id));
                let won = heat.is_some_and(|h| h.winner.as_deref() == Some(id));
                let wildcard = previous.is_some_and(|r| r.wildcards.iter().any(|w| w == id));
                let bye = previous.is_some_and(|r| r.byes.iter().any(|w| w == id));
                let (class, label) = standing.map_or(("muted", "drawn"), |s| seat_pill(s.outcome));
                let score = standing.map_or_else(
                    || "Awaiting start".into(),
                    |s| {
                        s.durable_at_ms.map_or_else(
                            || format!("{} pts", s.milestone_points),
                            |ms| format!("{:.1}s", ms as f64 / 1000.0),
                        )
                    },
                );
                let origin = if wildcard {
                    "Wildcard"
                } else if bye {
                    "Bye"
                } else {
                    ""
                };
                let _ = write!(
                    rows,
                    "<li class=\"bracket-seat{}\" data-player=\"{}\" data-wildcard=\"{wildcard}\"><div><strong>{}</strong><small>{}</small></div><div class=\"bracket-seat-result\"><span>{}</span><small class=\"{}\">{}</small></div></li>",
                    if won { " won" } else { "" },
                    escape(id),
                    escape(id),
                    origin,
                    escape(&score),
                    class,
                    if won { "Heat winner" } else { label }
                );
            }
            if rows.is_empty() {
                for seat in 1..=week.draw.heat_size {
                    let _ = write!(
                        rows,
                        "<li class=\"bracket-seat pending\"><strong>Seat {seat}</strong><small>{}</small></li>",
                        if stopped {
                            "Not played"
                        } else {
                            "To be decided"
                        }
                    );
                }
            }
            let outcome = heat.map_or_else(
                || {
                    if stopped {
                        "Round not reached".into()
                    } else {
                        "Awaiting the race".into()
                    }
                },
                crate::season::heat_outcome_label,
            );
            let links =
                week.heat_links
                    .get(&(shape.round, number))
                    .map_or_else(String::new, |slugs| {
                        slugs
                            .iter()
                            .enumerate()
                            .map(|(i, slug)| {
                                format!(
                                    "<a href=\"../../../matches/{}/\">{} ↗</a>",
                                    escape(slug),
                                    if slugs.len() == 1 {
                                        "Watch replay".into()
                                    } else {
                                        format!("Attempt {}", i + 1)
                                    }
                                )
                            })
                            .collect()
                    });
            let _ = write!(
                cards,
                "<article class=\"bracket-match{}\"><header><span class=\"eyebrow\">Heat {number}</span><span>{}</span></header><ul>{rows}</ul><footer><strong>{}</strong><div>{links}</div></footer></article>",
                if heat.is_some_and(|h| h.aborted) {
                    " aborted"
                } else {
                    ""
                },
                heat.filter(|h| h.attempts > 1)
                    .map_or_else(String::new, |h| format!("{} attempts", h.attempts)),
                escape(&outcome)
            );
        }
        let byes = result
            .map(|r| r.byes.as_slice())
            .or_else(|| (shape.round == 1).then_some(week.draw.first_round.byes.as_slice()))
            .unwrap_or_default();
        for id in byes {
            let _ = write!(
                cards,
                "<p class=\"bracket-bye\" data-player=\"{}\">{} · bye to next round</p>",
                escape(id),
                escape(id)
            );
        }
        if let Some(r) = result.filter(|r| !r.wildcards.is_empty()) {
            let _ = write!(
                cards,
                "<p class=\"bracket-wildcard\">Wildcards: {}</p>",
                escape(&r.wildcards.join(", "))
            );
        }
        let arena = week
            .draw
            .round_arenas
            .get(shape.round - 1)
            .and_then(|i| week.draw.arenas.get(*i))
            .map_or("Scenario pending", |a| a.arena_id.as_str());
        let _ = write!(
            columns,
            "<div class=\"bracket-column\" data-round=\"{}\"><header><span class=\"eyebrow\">{} entrants</span><h3>{}</h3><p>{}</p></header><div class=\"bracket-matches\">{cards}</div></div>",
            shape.round,
            shape.field,
            if shape.round == week.draw.shape.len() {
                "Final".into()
            } else {
                format!("Round {}", shape.round)
            },
            escape(&humanize(arena))
        );
    }
    format!(
        "<section class=\"tournament-bracket\" aria-label=\"Tournament bracket\"><div class=\"section-heading\"><h2>The road to the crown</h2><p>Solid lines: advances · dashed lines: wildcards</p></div><div class=\"bracket-viewport\" tabindex=\"0\" role=\"region\" aria-label=\"Bracket rounds, scroll horizontally on narrow desktop screens\"><div class=\"bracket-board\"><svg class=\"bracket-connectors\" aria-hidden=\"true\"></svg>{columns}</div></div></section><script>{BRACKET_SCRIPT}</script>"
    )
}

pub(super) fn spend(week: &WeekReport) -> String {
    week.summary.as_ref().map_or_else(String::new, |summary| {
        let total = summary.standings.iter().fold(0_u64, |sum, s| sum.saturating_add(s.cost_microusd));
        let recorded = if summary.standings.iter().all(|s| s.cost_complete == Some(true)) {
            season_money(total, replay_spend_missing(summary))
        } else {
            aoe_tui::format_usage_cost("", total, Some(false))
        };
        let cost = subscription_total(summary.standings.iter().any(|s| s.model.starts_with("opencode-go/")), recorded);
        format!("<p class=\"bracket-note\">Recorded tournament spend: {} · includes retained replay attempts; missing provider costs are not free inference.</p>", escape(&cost))
    })
}
