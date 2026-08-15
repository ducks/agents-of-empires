use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use aoe_domain::{ArenaManifest, CompetitorState, Event, EventEnvelope, MatchState};

use crate::RefereeError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildStanding {
    pub territory: String,
    pub state: CompetitorState,
    pub points: u64,
    pub passed: usize,
    pub durable_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOutcome {
    pub winner: Option<String>,
    pub reason: String,
    pub standings: Vec<BuildStanding>,
}

struct CompetitorRecord {
    state: CompetitorState,
    passed: HashSet<String>,
    points: u64,
    durable_at_ms: Option<u64>,
}

pub struct BuildReferee {
    match_state: MatchState,
    completion_milestone: String,
    stop_on_first_durable: bool,
    competitors: HashMap<String, CompetitorRecord>,
    sequence: u64,
    last_time_ms: u64,
    outcome: Option<BuildOutcome>,
}

impl BuildReferee {
    #[must_use]
    pub fn from_manifest(manifest: &ArenaManifest) -> Self {
        let build = manifest.build.as_ref().expect("validated build manifest");
        let competitors = manifest
            .territories
            .iter()
            .map(|territory| {
                (
                    territory.id.clone(),
                    CompetitorRecord {
                        state: CompetitorState::Preparing,
                        passed: HashSet::new(),
                        points: 0,
                        durable_at_ms: None,
                    },
                )
            })
            .collect();
        Self {
            match_state: MatchState::Preparing,
            completion_milestone: build.completion_milestone.clone(),
            stop_on_first_durable: build.stop_on_first_durable,
            competitors,
            sequence: 0,
            last_time_ms: 0,
            outcome: None,
        }
    }

    pub fn start(&mut self) -> Result<Vec<EventEnvelope>, RefereeError> {
        if self.match_state != MatchState::Preparing {
            return Err(RefereeError::MatchNotRunning);
        }
        self.match_state = MatchState::Running;
        let mut events = vec![self.emit(Event::MatchStateChanged {
            from: MatchState::Preparing,
            to: MatchState::Running,
        })];
        let mut ids: Vec<_> = self.competitors.keys().cloned().collect();
        ids.sort();
        for territory in ids {
            self.competitors
                .get_mut(&territory)
                .expect("known competitor")
                .state = CompetitorState::Building;
            events.push(self.emit(Event::CompetitorStateChanged {
                territory,
                from: CompetitorState::Preparing,
                to: CompetitorState::Building,
                reason: "SSH preflight passed".into(),
            }));
        }
        Ok(events)
    }

    pub fn record(&mut self, event: Event, elapsed_ms: u64) -> Result<EventEnvelope, RefereeError> {
        self.ensure_time(elapsed_ms)?;
        Ok(self.emit(event))
    }

    pub fn begin_milestone(
        &mut self,
        territory: &str,
        milestone: &str,
        elapsed_ms: u64,
    ) -> Result<Vec<EventEnvelope>, RefereeError> {
        self.ensure_running()?;
        self.ensure_time(elapsed_ms)?;
        let record = self
            .competitors
            .get_mut(territory)
            .ok_or_else(|| RefereeError::UnknownTerritory(territory.into()))?;
        let mut events = Vec::new();
        if record.state == CompetitorState::Building {
            record.state = CompetitorState::Verifying;
            events.push(self.emit(Event::CompetitorStateChanged {
                territory: territory.into(),
                from: CompetitorState::Building,
                to: CompetitorState::Verifying,
                reason: "milestone verification began".into(),
            }));
        }
        events.push(self.emit(Event::MilestoneEvaluationStarted {
            territory: territory.into(),
            milestone: milestone.into(),
        }));
        Ok(events)
    }

    pub fn pass_milestone(
        &mut self,
        territory: &str,
        milestone: &str,
        points: u64,
        evidence: serde_json::Value,
        elapsed_ms: u64,
    ) -> Result<Vec<EventEnvelope>, RefereeError> {
        self.ensure_running()?;
        self.ensure_time(elapsed_ms)?;
        let record = self
            .competitors
            .get_mut(territory)
            .ok_or_else(|| RefereeError::UnknownTerritory(territory.into()))?;
        if !record.passed.insert(milestone.into()) {
            return Ok(Vec::new());
        }
        record.points = record.points.saturating_add(points);
        let mut events = vec![self.emit(Event::MilestonePassed {
            territory: territory.into(),
            milestone: milestone.into(),
            points,
            evidence,
        })];
        if milestone == self.completion_milestone {
            let record = self.competitors.get_mut(territory).expect("competitor");
            record.state = CompetitorState::Durable;
            record.durable_at_ms = Some(elapsed_ms);
            events.push(self.emit(Event::CompetitorStateChanged {
                territory: territory.into(),
                from: CompetitorState::Verifying,
                to: CompetitorState::Durable,
                reason: "completion milestone passed".into(),
            }));
            events.push(self.emit(Event::DurableDeploymentCompleted {
                territory: territory.into(),
                elapsed_ms,
            }));
            if self.stop_on_first_durable {
                events.extend(self.finish_inner("first durable deployment".into(), elapsed_ms));
            }
        }
        Ok(events)
    }

    pub fn fail_milestone(
        &mut self,
        territory: &str,
        milestone: &str,
        category: &str,
        detail: &str,
        retryable: bool,
        elapsed_ms: u64,
    ) -> Result<Vec<EventEnvelope>, RefereeError> {
        self.ensure_running()?;
        self.ensure_time(elapsed_ms)?;
        if !self.competitors.contains_key(territory) {
            return Err(RefereeError::UnknownTerritory(territory.into()));
        }
        Ok(vec![self.emit(Event::MilestoneFailed {
            territory: territory.into(),
            milestone: milestone.into(),
            category: category.into(),
            detail: detail.into(),
            retryable,
        })])
    }

    pub fn finish(
        &mut self,
        reason: impl Into<String>,
        elapsed_ms: u64,
    ) -> Result<Vec<EventEnvelope>, RefereeError> {
        self.ensure_running()?;
        self.ensure_time(elapsed_ms)?;
        Ok(self.finish_inner(reason.into(), elapsed_ms))
    }

    pub fn abort(
        &mut self,
        reason: impl Into<String>,
        elapsed_ms: u64,
    ) -> Result<Vec<EventEnvelope>, RefereeError> {
        self.ensure_running()?;
        self.ensure_time(elapsed_ms)?;
        self.match_state = MatchState::Aborted;
        Ok(vec![
            self.emit(Event::MatchStateChanged {
                from: MatchState::Running,
                to: MatchState::Aborted,
            }),
            self.emit(Event::MatchFinished {
                winner: None,
                reason: reason.into(),
            }),
        ])
    }

    #[must_use]
    pub fn outcome(&self) -> Option<&BuildOutcome> {
        self.outcome.as_ref()
    }

    fn finish_inner(&mut self, reason: String, _elapsed_ms: u64) -> Vec<EventEnvelope> {
        let mut standings: Vec<_> = self
            .competitors
            .iter()
            .map(|(territory, record)| BuildStanding {
                territory: territory.clone(),
                state: record.state,
                points: record.points,
                passed: record.passed.len(),
                durable_at_ms: record.durable_at_ms,
            })
            .collect();
        standings.sort_by_key(|standing| {
            (
                standing.durable_at_ms.is_none(),
                standing.durable_at_ms.unwrap_or(u64::MAX),
                Reverse(standing.points),
                Reverse(standing.passed),
                standing.territory.clone(),
            )
        });
        let winner = standings.first().and_then(|leader| {
            let tied = standings.get(1).is_some_and(|runner_up| {
                leader.durable_at_ms == runner_up.durable_at_ms
                    && leader.points == runner_up.points
                    && leader.passed == runner_up.passed
            });
            (!tied).then(|| leader.territory.clone())
        });
        self.match_state = MatchState::Finished;
        self.outcome = Some(BuildOutcome {
            winner: winner.clone(),
            reason: reason.clone(),
            standings,
        });
        vec![
            self.emit(Event::MatchStateChanged {
                from: MatchState::Running,
                to: MatchState::Finished,
            }),
            self.emit(Event::MatchFinished { winner, reason }),
        ]
    }

    fn ensure_running(&self) -> Result<(), RefereeError> {
        if self.match_state == MatchState::Running {
            Ok(())
        } else {
            Err(RefereeError::MatchNotRunning)
        }
    }

    fn ensure_time(&mut self, elapsed_ms: u64) -> Result<(), RefereeError> {
        if elapsed_ms < self.last_time_ms {
            return Err(RefereeError::TimeMovedBackward {
                previous: self.last_time_ms,
                current: elapsed_ms,
            });
        }
        self.last_time_ms = elapsed_ms;
        Ok(())
    }

    fn emit(&mut self, event: Event) -> EventEnvelope {
        let envelope = EventEnvelope {
            schema_version: 1,
            sequence: self.sequence,
            elapsed_ms: self.last_time_ms,
            event,
        };
        self.sequence = self.sequence.saturating_add(1);
        envelope
    }
}
