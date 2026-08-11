use std::cmp::Reverse;
use std::collections::HashMap;

use aoe_domain::{
    ArenaManifest, Event, EventEnvelope, FailureSource, HealthPolicy, MatchState, TerritoryState,
};
use thiserror::Error;

use crate::HealthObservation;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RefereeError {
    #[error("unknown territory {0}")]
    UnknownTerritory(String),
    #[error("event time moved backward from {previous}ms to {current}ms")]
    TimeMovedBackward { previous: u64, current: u64 },
    #[error("match is not running")]
    MatchNotRunning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    pub territory: String,
    pub state: TerritoryState,
    pub uptime_ticks: u64,
    pub degraded_ms: u64,
    pub resources: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchOutcome {
    pub winner: Option<String>,
    pub reason: String,
    pub standings: Vec<Standing>,
}

struct TerritoryRecord {
    state: TerritoryState,
    policy: HealthPolicy,
    failure_streak: u32,
    recovery_streak: u32,
    recovery_deadline_ms: Option<u64>,
    last_accounted_ms: u64,
    degraded_ms: u64,
    uptime_ticks: u64,
    resources: u64,
}

/// Pure game-state authority fed by external observations.
pub struct Referee {
    match_state: MatchState,
    duration_ms: u64,
    healthy_resources_per_tick: u64,
    territories: HashMap<String, TerritoryRecord>,
    sequence: u64,
    last_time_ms: u64,
    outcome: Option<MatchOutcome>,
}

impl Referee {
    #[must_use]
    pub fn from_manifest(manifest: &ArenaManifest) -> Self {
        let budgets: HashMap<_, _> = manifest
            .agents
            .iter()
            .map(|agent| (agent.territory.as_str(), agent.budget.resource_units))
            .collect();
        let territories = manifest
            .territories
            .iter()
            .map(|territory| {
                (
                    territory.id.clone(),
                    TerritoryRecord {
                        state: TerritoryState::Provisioning,
                        policy: territory.service.health.clone(),
                        failure_streak: 0,
                        recovery_streak: 0,
                        recovery_deadline_ms: None,
                        last_accounted_ms: 0,
                        degraded_ms: 0,
                        uptime_ticks: 0,
                        resources: budgets.get(territory.id.as_str()).copied().unwrap_or(0),
                    },
                )
            })
            .collect();
        Self {
            match_state: MatchState::Preparing,
            duration_ms: manifest.rules.duration_seconds.saturating_mul(1000),
            healthy_resources_per_tick: manifest.rules.healthy_resources_per_tick,
            territories,
            sequence: 0,
            last_time_ms: 0,
            outcome: None,
        }
    }

    /// Start the match after preflight has confirmed every territory healthy.
    ///
    /// # Errors
    ///
    /// Returns an error if the match has already started or finished.
    pub fn start(&mut self) -> Result<Vec<EventEnvelope>, RefereeError> {
        if self.match_state != MatchState::Preparing {
            return Err(RefereeError::MatchNotRunning);
        }
        let mut events = vec![self.emit(Event::MatchStateChanged {
            from: MatchState::Preparing,
            to: MatchState::Running,
        })];
        self.match_state = MatchState::Running;
        let mut ids: Vec<_> = self.territories.keys().cloned().collect();
        ids.sort();
        for id in ids {
            if let Some(record) = self.territories.get_mut(&id) {
                record.state = TerritoryState::Healthy;
            }
            events.push(self.emit(Event::TerritoryStateChanged {
                territory: id,
                from: TerritoryState::Provisioning,
                to: TerritoryState::Healthy,
                reason: "preflight passed".into(),
            }));
        }
        Ok(events)
    }

    /// Apply one external health observation.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown territories, non-monotonic time, or a
    /// match that is not running.
    pub fn observe(
        &mut self,
        territory: &str,
        observation: HealthObservation,
        elapsed_ms: u64,
    ) -> Result<Vec<EventEnvelope>, RefereeError> {
        self.ensure_running()?;
        self.ensure_time(elapsed_ms)?;
        if !self.territories.contains_key(territory) {
            return Err(RefereeError::UnknownTerritory(territory.into()));
        }
        self.account_time(territory, elapsed_ms);
        let mut events = vec![self.emit(Event::HealthObserved {
            territory: territory.into(),
            healthy: observation.healthy,
            status: observation.status,
            latency_ms: observation.latency_ms,
            detail: observation.detail,
        })];

        let mut transition = None;
        let mut eliminate = false;
        {
            let Some(record) = self.territories.get_mut(territory) else {
                return Err(RefereeError::UnknownTerritory(territory.into()));
            };
            if record.state == TerritoryState::Eliminated {
                return Ok(events);
            }
            if record
                .recovery_deadline_ms
                .is_some_and(|deadline| elapsed_ms >= deadline)
                && record.state != TerritoryState::Healthy
            {
                eliminate = true;
            } else if observation.healthy {
                record.failure_streak = 0;
                match record.state {
                    TerritoryState::Degraded | TerritoryState::Recovering => {
                        record.recovery_streak = record.recovery_streak.saturating_add(1);
                        if record.recovery_streak >= record.policy.consecutive_failures {
                            transition =
                                Some((record.state, TerritoryState::Healthy, "durable recovery"));
                            record.state = TerritoryState::Healthy;
                            record.recovery_deadline_ms = None;
                            record.recovery_streak = 0;
                        } else if record.state == TerritoryState::Degraded {
                            transition = Some((
                                TerritoryState::Degraded,
                                TerritoryState::Recovering,
                                "healthy probe during recovery window",
                            ));
                            record.state = TerritoryState::Recovering;
                        }
                    }
                    _ => {}
                }
            } else {
                record.recovery_streak = 0;
                match record.state {
                    TerritoryState::Healthy => {
                        record.failure_streak = record.failure_streak.saturating_add(1);
                        if record.failure_streak >= record.policy.consecutive_failures {
                            transition = Some((
                                TerritoryState::Healthy,
                                TerritoryState::Degraded,
                                "failure threshold reached",
                            ));
                            record.state = TerritoryState::Degraded;
                            record.recovery_deadline_ms = Some(elapsed_ms.saturating_add(
                                record.policy.recovery_window_seconds.saturating_mul(1000),
                            ));
                        }
                    }
                    TerritoryState::Recovering => {
                        transition = Some((
                            TerritoryState::Recovering,
                            TerritoryState::Degraded,
                            "recovery did not survive verification",
                        ));
                        record.state = TerritoryState::Degraded;
                    }
                    _ => {}
                }
            }
        }
        if let Some((from, to, reason)) = transition {
            events.push(self.emit(Event::TerritoryStateChanged {
                territory: territory.into(),
                from,
                to,
                reason: reason.into(),
            }));
        }
        if eliminate {
            events.extend(self.eliminate(
                territory,
                FailureSource::Player,
                "recovery window expired",
                elapsed_ms,
            )?);
        }
        Ok(events)
    }

    /// Apply one economy tick and enforce recovery deadlines and match time.
    ///
    /// # Errors
    ///
    /// Returns an error for non-monotonic time or a non-running match.
    pub fn tick(&mut self, elapsed_ms: u64) -> Result<Vec<EventEnvelope>, RefereeError> {
        self.ensure_running()?;
        self.ensure_time(elapsed_ms)?;
        let mut ids: Vec<_> = self.territories.keys().cloned().collect();
        ids.sort();
        let mut events = Vec::new();
        for id in &ids {
            self.account_time(id, elapsed_ms);
            let (healthy, expired, remaining) = {
                let Some(record) = self.territories.get_mut(id) else {
                    continue;
                };
                let healthy = record.state == TerritoryState::Healthy;
                if healthy {
                    record.uptime_ticks = record.uptime_ticks.saturating_add(1);
                    record.resources = record
                        .resources
                        .saturating_add(self.healthy_resources_per_tick);
                }
                let expired = record
                    .recovery_deadline_ms
                    .is_some_and(|deadline| elapsed_ms >= deadline)
                    && matches!(
                        record.state,
                        TerritoryState::Degraded | TerritoryState::Recovering
                    );
                (healthy, expired, record.resources)
            };
            if healthy && self.healthy_resources_per_tick > 0 {
                events.push(self.emit(Event::ResourcesChanged {
                    territory: id.clone(),
                    delta: i64::try_from(self.healthy_resources_per_tick).unwrap_or(i64::MAX),
                    remaining,
                    reason: "healthy tick".into(),
                }));
            }
            if expired {
                events.extend(self.eliminate(
                    id,
                    FailureSource::Player,
                    "recovery window expired",
                    elapsed_ms,
                )?);
            }
        }
        if elapsed_ms >= self.duration_ms && self.match_state == MatchState::Running {
            events.extend(self.finish("match timer expired", elapsed_ms));
        }
        Ok(events)
    }

    /// Charge abstract resources to one territory.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown territories or a non-running match.
    pub fn charge(
        &mut self,
        territory: &str,
        units: u64,
        elapsed_ms: u64,
    ) -> Result<EventEnvelope, RefereeError> {
        self.ensure_running()?;
        self.ensure_time(elapsed_ms)?;
        let record = self
            .territories
            .get_mut(territory)
            .ok_or_else(|| RefereeError::UnknownTerritory(territory.into()))?;
        record.resources = record.resources.saturating_sub(units);
        let remaining = record.resources;
        Ok(self.emit(Event::ResourcesChanged {
            territory: territory.into(),
            delta: -i64::try_from(units).unwrap_or(i64::MAX),
            remaining,
            reason: "agent inference".into(),
        }))
    }

    /// Record a failure outside player control without changing territory
    /// health or scoring it as a loss.
    ///
    /// # Errors
    ///
    /// Returns an error when event time moves backward.
    pub fn infrastructure_failure(
        &mut self,
        component: impl Into<String>,
        source: FailureSource,
        detail: impl Into<String>,
        elapsed_ms: u64,
    ) -> Result<EventEnvelope, RefereeError> {
        self.ensure_time(elapsed_ms)?;
        Ok(self.emit(Event::InfrastructureFailure {
            component: component.into(),
            source,
            detail: detail.into(),
        }))
    }

    /// Record a controller-observed event in the referee's canonical sequence.
    ///
    /// # Errors
    ///
    /// Returns an error when event time moves backward.
    pub fn record(&mut self, event: Event, elapsed_ms: u64) -> Result<EventEnvelope, RefereeError> {
        self.ensure_time(elapsed_ms)?;
        Ok(self.emit(event))
    }

    /// Abort a running match while retaining a replayable terminal event.
    ///
    /// # Errors
    ///
    /// Returns an error for non-monotonic time or a match that is not running.
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
    pub fn outcome(&self) -> Option<&MatchOutcome> {
        self.outcome.as_ref()
    }

    #[must_use]
    pub fn territory_state(&self, territory: &str) -> Option<TerritoryState> {
        self.territories.get(territory).map(|record| record.state)
    }

    fn eliminate(
        &mut self,
        territory: &str,
        source: FailureSource,
        detail: &str,
        elapsed_ms: u64,
    ) -> Result<Vec<EventEnvelope>, RefereeError> {
        self.account_time(territory, elapsed_ms);
        let record = self
            .territories
            .get_mut(territory)
            .ok_or_else(|| RefereeError::UnknownTerritory(territory.into()))?;
        let from = record.state;
        record.state = TerritoryState::Eliminated;
        record.recovery_deadline_ms = None;
        let mut events = vec![
            self.emit(Event::TerritoryStateChanged {
                territory: territory.into(),
                from,
                to: TerritoryState::Eliminated,
                reason: detail.into(),
            }),
            self.emit(Event::TerritoryEliminated {
                territory: territory.into(),
                source,
                detail: detail.into(),
            }),
        ];
        let active = self
            .territories
            .values()
            .filter(|record| record.state != TerritoryState::Eliminated)
            .count();
        if active <= 1 && self.territories.len() > 1 {
            events.extend(self.finish("last territory standing", elapsed_ms));
        }
        Ok(events)
    }

    fn finish(&mut self, reason: &str, elapsed_ms: u64) -> Vec<EventEnvelope> {
        for id in self.territories.keys().cloned().collect::<Vec<_>>() {
            self.account_time(&id, elapsed_ms);
        }
        let mut standings = self.standings();
        standings.sort_by_key(|standing| {
            (
                standing.state == TerritoryState::Eliminated,
                Reverse(standing.uptime_ticks),
                Reverse(standing.resources),
                standing.degraded_ms,
                standing.territory.clone(),
            )
        });
        let winner = standings
            .first()
            .filter(|standing| standing.state != TerritoryState::Eliminated)
            .map(|standing| standing.territory.clone());
        self.match_state = MatchState::Finished;
        self.outcome = Some(MatchOutcome {
            winner: winner.clone(),
            reason: reason.into(),
            standings,
        });
        vec![
            self.emit(Event::MatchStateChanged {
                from: MatchState::Running,
                to: MatchState::Finished,
            }),
            self.emit(Event::MatchFinished {
                winner,
                reason: reason.into(),
            }),
        ]
    }

    fn standings(&self) -> Vec<Standing> {
        self.territories
            .iter()
            .map(|(territory, record)| Standing {
                territory: territory.clone(),
                state: record.state,
                uptime_ticks: record.uptime_ticks,
                degraded_ms: record.degraded_ms,
                resources: record.resources,
            })
            .collect()
    }

    fn account_time(&mut self, territory: &str, elapsed_ms: u64) {
        if let Some(record) = self.territories.get_mut(territory) {
            if matches!(
                record.state,
                TerritoryState::Degraded | TerritoryState::Recovering
            ) {
                record.degraded_ms = record
                    .degraded_ms
                    .saturating_add(elapsed_ms.saturating_sub(record.last_accounted_ms));
            }
            record.last_accounted_ms = elapsed_ms;
        }
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
        let sequence = self.sequence;
        self.sequence = self.sequence.saturating_add(1);
        EventEnvelope {
            schema_version: 1,
            sequence,
            elapsed_ms: self.last_time_ms,
            event,
        }
    }
}
