//! Constraints and indexes for the operational data model. Every statement is
//! idempotent, so all roles run this at startup. Only uniqueness constraints
//! and indexes are used, which Neo4j Community supports.
//!
//! The legacy `:Incident`, `:IncidentAuditEvent` and `:ResourceReservation`
//! schema is left untouched while both pipelines coexist.

use neo4rs::query;

use super::Neo4jClient;

/// Names of the uniqueness constraints created here (checked by tests).
pub const OPS_CONSTRAINTS: &[&str] = &[
    "ops_report_id_unique",
    "ops_report_request_id_unique",
    "ops_task_id_unique",
    "ops_vehicle_id_unique",
    "ops_reservation_id_unique",
    "ops_reservation_active_vehicle_unique",
    "ops_reservation_active_task_unique",
    "ops_audit_event_id_unique",
    "ops_outbox_event_id_unique",
    "ops_processed_event_unique",
    "ops_overlay_state_dataset_unique",
    "ops_lease_fence_name_unique",
    "ops_incident_id_unique",
];

const STATEMENTS: &[&str] = &[
    // Identity
    "CREATE CONSTRAINT ops_report_id_unique IF NOT EXISTS \
     FOR (r:Report) REQUIRE r.report_id IS UNIQUE",
    // D-1: a repeated request_id is a retry of the same report
    "CREATE CONSTRAINT ops_report_request_id_unique IF NOT EXISTS \
     FOR (r:Report) REQUIRE r.request_id IS UNIQUE",
    "CREATE CONSTRAINT ops_task_id_unique IF NOT EXISTS \
     FOR (t:Task) REQUIRE t.task_id IS UNIQUE",
    "CREATE CONSTRAINT ops_vehicle_id_unique IF NOT EXISTS \
     FOR (v:Vehicle) REQUIRE v.vehicle_id IS UNIQUE",
    "CREATE CONSTRAINT ops_reservation_id_unique IF NOT EXISTS \
     FOR (r:Reservation) REQUIRE r.reservation_id IS UNIQUE",
    // Never overbook: these keys exist only while a reservation is active,
    // so a vehicle (or a task, per resource kind) can hold at most one.
    "CREATE CONSTRAINT ops_reservation_active_vehicle_unique IF NOT EXISTS \
     FOR (r:Reservation) REQUIRE r.active_vehicle_key IS UNIQUE",
    "CREATE CONSTRAINT ops_reservation_active_task_unique IF NOT EXISTS \
     FOR (r:Reservation) REQUIRE r.active_task_key IS UNIQUE",
    "CREATE CONSTRAINT ops_audit_event_id_unique IF NOT EXISTS \
     FOR (e:AuditEvent) REQUIRE e.event_id IS UNIQUE",
    "CREATE CONSTRAINT ops_outbox_event_id_unique IF NOT EXISTS \
     FOR (e:OutboxEvent) REQUIRE e.event_id IS UNIQUE",
    "CREATE CONSTRAINT ops_processed_event_unique IF NOT EXISTS \
     FOR (p:ProcessedEvent) REQUIRE (p.consumer, p.event_id) IS UNIQUE",
    "CREATE CONSTRAINT ops_overlay_state_dataset_unique IF NOT EXISTS \
     FOR (o:OverlayState) REQUIRE o.dataset IS UNIQUE",
    // New-pipeline incidents use :OpsIncident while the legacy console still
    // reads every :Incident (renamed to :Incident at legacy removal, M9).
    "CREATE CONSTRAINT ops_incident_id_unique IF NOT EXISTS \
     FOR (i:OpsIncident) REQUIRE i.incident_id IS UNIQUE",
    "CREATE POINT INDEX ops_ops_incident_location IF NOT EXISTS \
     FOR (i:OpsIncident) ON (i.location)",
    "CREATE INDEX ops_ops_incident_status_created IF NOT EXISTS \
     FOR (i:OpsIncident) ON (i.status, i.created_at)",
    "CREATE INDEX ops_ops_incident_junction IF NOT EXISTS \
     FOR (i:OpsIncident) ON (i.junction_id)",
    // Dispatcher callback list (UNKNOWN urgency, M3b)
    "CREATE INDEX ops_ops_incident_callback IF NOT EXISTS \
     FOR (i:OpsIncident) ON (i.callback_required, i.review_rank)",
    // Hazard records (M4): one active hazard per segment, idempotent commands
    "CREATE CONSTRAINT ops_hazard_id_unique IF NOT EXISTS \
     FOR (h:Hazard) REQUIRE h.hazard_id IS UNIQUE",
    "CREATE CONSTRAINT ops_hazard_request_id_unique IF NOT EXISTS \
     FOR (h:Hazard) REQUIRE h.request_id IS UNIQUE",
    "CREATE CONSTRAINT ops_hazard_active_key_unique IF NOT EXISTS \
     FOR (h:Hazard) REQUIRE h.active_key IS UNIQUE",
    "CREATE INDEX ops_hazard_segment IF NOT EXISTS \
     FOR (h:Hazard) ON (h.dataset, h.segment_id)",
    // Fencing tokens for leader-only writes (outbox relay, matcher)
    "CREATE CONSTRAINT ops_lease_fence_name_unique IF NOT EXISTS \
     FOR (f:LeaseFence) REQUIRE f.name IS UNIQUE",
    // Spatial
    "CREATE POINT INDEX ops_vehicle_position IF NOT EXISTS \
     FOR (v:Vehicle) ON (v.position)",
    "CREATE POINT INDEX ops_incident_location IF NOT EXISTS \
     FOR (i:Incident) ON (i.location)",
    // Lookups
    "CREATE INDEX ops_report_device_received IF NOT EXISTS \
     FOR (r:Report) ON (r.device_id, r.received_at)",
    "CREATE INDEX ops_task_status IF NOT EXISTS FOR (t:Task) ON (t.status)",
    "CREATE INDEX ops_vehicle_status IF NOT EXISTS FOR (v:Vehicle) ON (v.status)",
    "CREATE INDEX ops_reservation_status IF NOT EXISTS \
     FOR (r:Reservation) ON (r.status)",
    "CREATE INDEX ops_outbox_unpublished IF NOT EXISTS \
     FOR (e:OutboxEvent) ON (e.published, e.created_at)",
    "CREATE INDEX ops_audit_aggregate IF NOT EXISTS \
     FOR (e:AuditEvent) ON (e.aggregate_id)",
];

/// Create every operational constraint and index if missing.
///
/// Several processes may run this at the same moment (roles starting
/// together). `IF NOT EXISTS` does not stop two concurrent creations of the
/// same new rule from colliding: the loser fails with an "already exists"
/// error, which here means the rule is in place, so it is treated as success.
pub async fn ensure_ops_schema(
    neo4j: &Neo4jClient,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for statement in STATEMENTS {
        match neo4j.graph.run(query(statement)).await {
            Ok(()) => {}
            Err(neo4rs::Error::Neo4j(e)) if is_already_exists(e.code()) => {}
            Err(neo4rs::Error::UnexpectedMessage(m)) if ALREADY_EXISTS.iter().any(|c| m.contains(c)) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

const ALREADY_EXISTS: &[&str] = &[
    "Neo.ClientError.Schema.EquivalentSchemaRuleAlreadyExists",
    "Neo.ClientError.Schema.IndexAlreadyExists",
    "Neo.ClientError.Schema.ConstraintAlreadyExists",
];

fn is_already_exists(code: &str) -> bool {
    ALREADY_EXISTS.contains(&code)
}
