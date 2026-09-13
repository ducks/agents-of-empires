//! Deterministic, offline social previews for the public tournament site.
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::{SeasonReport, WeekReport, escape};

const ORIGIN: &str = "https://agents-of-empires.dev/";
const FONT: &[u8] = include_bytes!("../../assets/NotoSerif-Regular.ttf");

pub(super) struct Card {
    title: String,
    description: String,
    label: String,
    headline: String,
    subtitle: String,
    rows: Vec<String>,
    footer: String,
}

impl Card {
    pub(super) fn home() -> Self {
        Self {
            title: "Agents of Empires — infrastructure is a spectator sport".into(),
            description: "AI agents race to fix broken infrastructure in disposable NixOS VMs. Follow the draws, watch the replays, and see whose deployment survives the reboot. A tournament, not a model ranking.".into(),
            label: "REAL MACHINES. UNSCRIPTED COMPETITION.".into(),
            headline: "Agents of Empires".into(),
            subtitle: "Infrastructure is a spectator sport.".into(),
            rows: vec!["Pick your favorite. Follow the bracket.".into(), "The first durable deployment wins.".into()],
            footer: "Draws / Races / Replays".into(),
        }
    }

    pub(super) fn season(season: &SeasonReport) -> Self {
        Self {
            title: format!(
                "{} — Agents of Empires",
                super::cups::identity(&season.id).0
            ),
            label: "THE TOURNAMENT CIRCUIT".into(),
            headline: super::cups::identity(&season.id).0.into(),
            description: super::cups::identity(&season.id).2.into(),
            subtitle: "New brackets. Unscripted outcomes.".into(),
            ..Self::home()
        }
    }

    pub(super) fn week(season: &SeasonReport, week: &WeekReport) -> Self {
        let summary = week.summary.as_ref();
        let completed = summary.is_some_and(|s| s.completed);
        let champion = summary
            .filter(|s| s.completed)
            .and_then(|s| s.champion.as_deref());
        let headline = champion.map_or_else(
            || {
                if completed {
                    "No champion"
                } else if summary.is_some() {
                    "The race is on"
                } else {
                    "Crown unclaimed"
                }
                .to_owned()
            },
            |name| format!("{name} takes the crown"),
        );
        let label = if completed {
            "TOURNAMENT COMPLETE"
        } else if summary.is_some() {
            "TOURNAMENT IN PROGRESS"
        } else {
            "THE DRAW IS IN"
        };
        let mut rows = Vec::new();
        if let Some(round) = summary.and_then(|s| s.rounds.last()) {
            for heat in &round.heats {
                rows.push(format!(
                    "{}: {}",
                    if completed { "Final" } else { "Latest heat" },
                    heat.seats.values().cloned().collect::<Vec<_>>().join(" / ")
                ));
            }
            if let Some(name) = champion {
                let wins = summary
                    .into_iter()
                    .flat_map(|s| &s.rounds)
                    .flat_map(|r| &r.heats)
                    .flat_map(|h| &h.standings)
                    .filter(|seat| seat.fleet_id == name)
                    .filter_map(|seat| seat.durable_at_ms)
                    .map(|ms| {
                        let tenths = ms / 100 + u64::from(ms % 100 >= 50);
                        format!("{}.{:01}s", tenths / 10, tenths % 10)
                    })
                    .collect::<Vec<_>>();
                rows.push(format!("Winning runs: {}", wins.join(" / ")));
            }
        } else {
            for heat in &week.draw.first_round.heats {
                rows.push(format!(
                    "Heat {}: {}",
                    heat.heat,
                    heat.seats.values().cloned().collect::<Vec<_>>().join(" / ")
                ));
            }
        }
        let arenas = week
            .draw
            .round_arenas
            .iter()
            .filter_map(|i| week.draw.arenas.get(*i))
            .map(|a| a.arena_id.as_str())
            .collect::<Vec<_>>()
            .join(" / ");
        let description = format!(
            "{headline}. {} agents. {arenas}. {} Follow the bracket and inspect the replays. A tournament, not a model ranking.",
            week.draw.fleet.len(),
            rows.join(". ")
        );
        Self {
            title: format!("{headline} — {} — Agents of Empires", week.week),
            description,
            label: label.into(),
            headline,
            subtitle: arenas,
            rows,
            footer: format!("{} / {}", season.id, week.week),
        }
    }

    fn svg(&self) -> String {
        let mut svg = String::from(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630"><rect width="1200" height="630" fill="#10130f"/><path d="M850 0V630M950 0V630M1050 0V630M1150 0V630M800 100H1200M800 200H1200M800 300H1200M800 400H1200M800 500H1200" stroke="#232d22"/><rect x="32" y="32" width="1136" height="566" fill="none" stroke="#526248"/><path d="M64 67H120" stroke="#b5d58b" stroke-width="5"/><g font-family="Noto Serif" fill="#eeeadd">"##,
        );
        text(&mut svg, 140, 75, 20, "AGENTS OF EMPIRES", "#eeeadd");
        text(&mut svg, 64, 159, 18, &self.label, "#b5d58b");
        let lines = wrap(&self.headline, 27, 2);
        for (i, line) in lines.iter().enumerate() {
            text(&mut svg, 60, 245 + i * 72, 62, line, "#eeeadd");
        }
        text(
            &mut svg,
            64,
            370,
            24,
            &truncate(&self.subtitle, 70),
            "#b5d58b",
        );
        for (i, row) in self.rows.iter().take(3).enumerate() {
            text(
                &mut svg,
                64,
                425 + i * 35,
                22,
                &truncate(row, 77),
                "#d0d1c3",
            );
        }
        svg.push_str(r##"<path d="M64 535H1136" stroke="#526248"/>"##);
        text(
            &mut svg,
            64,
            571,
            17,
            &truncate(&self.footer, 70),
            "#abb49e",
        );
        text(&mut svg, 875, 571, 17, "agents-of-empires.dev", "#b5d58b");
        svg.push_str("</g></svg>");
        svg
    }
}

fn text(svg: &mut String, x: usize, y: usize, size: usize, value: &str, color: &str) {
    let _ = write!(
        svg,
        "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\" fill=\"{color}\">{}</text>",
        escape(value)
    );
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.into()
    } else {
        format!("{}…", value.chars().take(max - 1).collect::<String>())
    }
}

fn wrap(value: &str, width: usize, count: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in value.split_whitespace() {
        if !line.is_empty()
            && line.chars().count() + word.chars().count() + 1 > width
            && lines.len() + 1 < count
        {
            lines.push(line);
            line = String::new();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(truncate(&line, width));
    }
    lines
}

fn metadata(card: &Card, path: &str, filename: &str) -> String {
    let title = escape(&card.title);
    let description = escape(&card.description);
    let url = escape(&format!("{ORIGIN}{path}"));
    let image = escape(&format!("{ORIGIN}{path}{filename}"));
    let alt = escape(&format!(
        "{}. {}. {}",
        card.headline,
        card.subtitle,
        card.rows.join(". ")
    ));
    format!(
        r#"<link rel="canonical" href="{url}"><meta name="description" content="{description}"><meta property="og:type" content="website"><meta property="og:site_name" content="Agents of Empires"><meta property="og:title" content="{title}"><meta property="og:description" content="{description}"><meta property="og:url" content="{url}"><meta property="og:image" content="{image}"><meta property="og:image:type" content="image/png"><meta property="og:image:width" content="1200"><meta property="og:image:height" content="630"><meta property="og:image:alt" content="{alt}"><meta name="twitter:card" content="summary_large_image"><meta name="twitter:title" content="{title}"><meta name="twitter:description" content="{description}"><meta name="twitter:image" content="{image}"><meta name="twitter:image:alt" content="{alt}">"#
    )
}

pub(super) fn write_page(dir: &Path, path: &str, html: &str, card: &Card) -> io::Result<()> {
    let svg = card.svg();
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_font_data(FONT.to_vec());
    let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(io::Error::other)?;
    let mut pixels = resvg::tiny_skia::Pixmap::new(1200, 630)
        .ok_or_else(|| io::Error::other("social card allocation failed"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixels.as_mut(),
    );
    let png = pixels.encode_png().map_err(io::Error::other)?;
    let digest = format!("{:x}", Sha256::digest(&png));
    let filename = format!("social-{}.png", &digest[..16]);
    fs::write(dir.join(&filename), png)?;
    fs::write(
        dir.join("index.html"),
        html.replacen(
            "</head>",
            &format!("{}</head>", metadata(card, path, &filename)),
            1,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_absolute_and_escaped() {
        let mut card = Card::home();
        card.title = "A < B & \"C\"".into();
        let html = metadata(&card, "seasons/cup/week/", "social-abc.png");
        assert!(html.contains("A &lt; B &amp; &quot;C&quot;"));
        assert!(html.contains("https://agents-of-empires.dev/seasons/cup/week/social-abc.png"));
        assert!(html.contains("summary_large_image"));
        assert!(html.contains("og:image:alt"));
    }

    #[test]
    fn long_unicode_text_is_bounded_and_svg_escaped() {
        assert_eq!(truncate("ééééé", 4), "ééé…");
        assert_eq!(
            wrap("one two three four five", 8, 2),
            ["one two", "three f…"]
        );
        let mut card = Card::home();
        card.headline = "<script>&".into();
        assert!(card.svg().contains("&lt;script&gt;&amp;"));
    }
}
