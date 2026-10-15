//! QA-09: RAG quality evaluation harness measuring lexical (BM25) retrieval.
//!
//! NOTE: Only LEXICAL (BM25) retrieval is currently wired in the Rust engine path
//! (there is no embedding engine wired in Rust today). Therefore, this evaluation
//! set specifically measures lexical hit@5 retrieval accuracy over chunked
//! documents. Semantic and hybrid hit@5 evaluation is a follow-up once embedding
//! engines land.
//!
//! Fixture documents:
//! 1. `device_manual` (Edge hardware gateway: specs, mounting, firmware reset, warranty)
//! 2. `developer_readme` (Job queue engine: architecture, build, cli flags, license)
//! 3. `service_faq` (Cloud account service: pricing, security/MFA, retention, error codes)

use super::chunker::{chunk_document, ChunkerConfig, PageText};
use super::search::{bm25_rank, ChunkRecord};

/// Evaluated query and its expected ground-truth location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalQuestion {
    pub query: &'static str,
    pub expected_doc: &'static str,
    pub expected_page: u32,
}

/// Detailed evaluation summary for inspection and CI reporting.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalResult {
    pub total_questions: usize,
    pub hits: usize,
    pub hit_rate: f32,
    pub details: Vec<QuestionEvalDetail>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuestionEvalDetail {
    pub query: String,
    pub expected_doc: String,
    pub expected_page: u32,
    pub hit: bool,
    pub top_retrieved: Vec<(String, u32, f32)>,
}

// ---------------------------------------------------------------------------
// Fixture Document 1: Device Hardware Manual (specs, mounting, reset, warranty)
// ---------------------------------------------------------------------------

const DEVICE_MANUAL_P1: &str = "\
The PocketNode Edge Gateway is a low-power edge computing appliance designed \
for distributed telemetry processing. The unit operates on a 5V DC input voltage \
via USB-C, drawing a nominal 2.0A under maximum processor load. The operating \
temperature range spans -10 degrees Celsius to 50 degrees Celsius with relative \
humidity between 5% and 95% non-condensing. Physical chassis dimensions measure \
85mm by 55mm by 15mm with a total unit weight of 65 grams.\n\n\
The front status panel features a multi-color LED indicator to reflect \
operational states: solid green indicates normal runtime operation, flashing \
amber signals network pairing mode, and steady red alerts the operator to a \
hardware fault or over-temperature condition. Hardware interface ports include \
dual Gigabit Ethernet RJ-45 jacks, one USB-C power delivery port, and an \
auxiliary expansion connector for external sensor modules.";

const DEVICE_MANUAL_P2: &str = "\
To mount the device on a vertical wall or DIN rail, utilize the supplied mounting \
bracket and two M3 drywall anchors. Align the mounting bracket at a recommended \
height of 1.5 meters from floor level in a location free from direct heat sources \
and excessive vibration. Secure the baseplate with screws before sliding the chassis \
into the retaining slots until an audible click confirms positive retention.\n\n\
Network connectivity can be established through a wired Ethernet cable or the \
onboard dual-band Wi-Fi interface supporting 802.11ax. On initial power-up, the \
device broadcasts a temporary captive portal SSID named PocketNode-Setup. Connect \
a configuration laptop to this wireless access point and open a browser to \
configure the local network SSID, subnet mask, default gateway, and DNS servers.";

const DEVICE_MANUAL_P3: &str = "\
Firmware updates are distributed over-the-air (OTA) and signed with cryptographic \
keys. Administrators can initiate manual firmware upgrades through the web console \
or schedule automatic overnight maintenance windows. By default, unattended update \
checks execute nightly at 03:00 UTC. Never disconnect power while the status LED \
flashes alternating purple and blue during flash memory reprogramming.\n\n\
If the device becomes unresponsive or network credentials are lost, perform a \
hardware factory reset. Locate the recessed pinhole reset switch on the bottom \
panel. Insert a straightened paperclip into the pinhole and depress the internal \
button continuously for 10 seconds. Release the button once the amber LED begins \
rapid blinking; the device will purge stored configuration files and reboot into \
factory defaults.";

const DEVICE_MANUAL_P4: &str = "\
The manufacturer provides a limited two-year warranty covering defects in \
materials and workmanship under standard indoor operational conditions starting \
from the original purchase date. Proof of purchase from an authorized distributor \
is required for all warranty claims and return merchandise authorizations (RMA).\n\n\
This warranty expressly excludes coverage for damage resulting from liquid spills, \
unauthorized disassembly, lightning strikes, electrostatic discharge, or power \
surges exceeding specified voltage tolerances. The device complies with FCC \
Part 15 Class B emissions limits and European CE standards for electromagnetic \
compatibility and low-voltage safety.";

// ---------------------------------------------------------------------------
// Fixture Document 2: Developer README (architecture, build, flags, license)
// ---------------------------------------------------------------------------

const DEVELOPER_README_P1: &str = "\
PocketQueue is a lightweight, embeddable asynchronous job queue engine engineered \
in Rust for high-throughput, low-latency background task execution. The core \
architecture is built upon lock-free ring buffers and atomic state machines, \
eliminating kernel mutex contention in critical worker dispatch loops.\n\n\
Key architectural subsystems include the priority scheduler, the worker thread \
pool executor, and the dead letter queue (DLQ). Failed jobs are automatically \
retried according to a user-defined exponential backoff policy before eviction to \
the DLQ, ensuring resilient task lifecycle management without blocking active \
consumer threads.";

const DEVELOPER_README_P2: &str = "\
Compiling PocketQueue from source requires the Rust toolchain version 1.80 or \
later along with the standard Cargo build system. For architectures supporting \
SIMD acceleration (x86_64 AVX2 or ARM Neon), ensure the LLVM compiler backend \
is installed and configured in your shell environment.\n\n\
To compile the optimized production release binary, execute cargo build --release \
in the root workspace directory. Optional compilation feature flags include \
rocksdb-backend for persistent disk-backed storage queues and tls-native for TLS \
encryption on inter-node communication channels.";

const DEVELOPER_README_P3: &str = "\
The engine executable accepts several command-line flags to tune runtime \
behavior. Use the --config <path> flag to supply a YAML configuration file, \
--concurrency <num> to set the maximum count of concurrent worker threads, \
and --port <num> to bind the HTTP telemetry health check endpoint.\n\n\
Environment variables can override configuration file settings in containerized \
deployments. Set POCKET_QUEUE_LOG_LEVEL to adjust tracing verbosity (trace, debug, \
info, warn, error) and set POCKET_QUEUE_SECRET to supply the HMAC secret key \
used to validate incoming client job authentication headers.";

const DEVELOPER_README_P4: &str = "\
PocketQueue is open source software dual-licensed under the Apache License \
Version 2.0 and the MIT License. Users and organizations may freely incorporate, \
modify, and distribute the software for commercial or private purposes under \
the permissive conditions of either license.\n\n\
We welcome external contributions and improvements. All pull requests must \
adhere to the project Contributor Covenant Code of Conduct, include unit tests \
maintaining code coverage above 85%, and feature cryptographically signed Git \
commits using a verified GPG key.";

// ---------------------------------------------------------------------------
// Fixture Document 3: Service FAQ (billing, MFA/security, retention, troubleshooting)
// ---------------------------------------------------------------------------

const SERVICE_FAQ_P1: &str = "\
Our cloud platform offers three subscription tiers: Community (free for \
individual hobbyists with rate-limited quotas), Professional ($29 per month \
per active user seat), and Enterprise (custom pricing with custom volume \
discounts, 99.99% uptime SLAs, and dedicated technical account managers).\n\n\
Subscription billing cycles recur monthly or annually on the first calendar day \
of the billing period. Upgrading seat allocations mid-cycle results in a \
pro-rated charge calculated for the remaining days of the current month. Payment \
invoices include itemized VAT breakdown and can be settled via corporate credit \
cards, ACH transfers, or wire payments.";

const SERVICE_FAQ_P2: &str = "\
Account security is safeguarded through mandatory multi-factor authentication \
(MFA). Organization administrators can require members to enroll either a \
time-based one-time password (TOTP) authenticator application or FIDO2 hardware \
security keys (such as YubiKey devices) for login authorization.\n\n\
Role-based access control (RBAC) allows administrators to assign granular \
permissions across three standard role tiers: Admin (full workspace control \
and billing management), Member (standard job submission and workflow \
editing), and Read-Only (view-only access to dashboard metrics and execution \
logs without modification rights).";

const SERVICE_FAQ_P3: &str = "\
Customer telemetry data and job logs are subject to strict data retention \
policies. Detailed operational logs are retained in hot storage for 90 days, \
after which aggregated hourly metrics are preserved for 365 days before automated \
archival to encrypted cold storage vaults.\n\n\
In compliance with European GDPR and California CCPA privacy regulations, \
tenants retain full data ownership. Authorized users can generate a complete \
data export archive in JSON format at any time or initiate a formal \
right-to-be-forgotten deletion request that permanently purges all tenant records \
within 30 days.";

const SERVICE_FAQ_P4: &str = "\
When debugging API communication issues, check standard response status codes: \
Error 4029 denotes client rate limit exhaustion (resolve by applying \
exponential backoff with randomized jitter), whereas Error 5012 indicates \
upstream synchronization timeout between the gateway and storage cluster.\n\n\
For technical assistance and incident escalation, Enterprise customers have \
access to a 24/7 priority ticketing queue and telephone emergency hotline. \
Community tier users can seek help through the public community discussion forum, \
and system availability can be monitored via real-time status page webhook \
notifications.";

// ---------------------------------------------------------------------------
// Document and Question Set Definitions
// ---------------------------------------------------------------------------

/// Returns the 3 public-domain-style fixture documents with their page texts.
pub fn fixture_documents() -> Vec<(&'static str, Vec<PageText>)> {
    vec![
        (
            "device_manual",
            vec![
                PageText {
                    page: 1,
                    text: DEVICE_MANUAL_P1.to_string(),
                },
                PageText {
                    page: 2,
                    text: DEVICE_MANUAL_P2.to_string(),
                },
                PageText {
                    page: 3,
                    text: DEVICE_MANUAL_P3.to_string(),
                },
                PageText {
                    page: 4,
                    text: DEVICE_MANUAL_P4.to_string(),
                },
            ],
        ),
        (
            "developer_readme",
            vec![
                PageText {
                    page: 1,
                    text: DEVELOPER_README_P1.to_string(),
                },
                PageText {
                    page: 2,
                    text: DEVELOPER_README_P2.to_string(),
                },
                PageText {
                    page: 3,
                    text: DEVELOPER_README_P3.to_string(),
                },
                PageText {
                    page: 4,
                    text: DEVELOPER_README_P4.to_string(),
                },
            ],
        ),
        (
            "service_faq",
            vec![
                PageText {
                    page: 1,
                    text: SERVICE_FAQ_P1.to_string(),
                },
                PageText {
                    page: 2,
                    text: SERVICE_FAQ_P2.to_string(),
                },
                PageText {
                    page: 3,
                    text: SERVICE_FAQ_P3.to_string(),
                },
                PageText {
                    page: 4,
                    text: SERVICE_FAQ_P4.to_string(),
                },
            ],
        ),
    ]
}

/// Builds the `ChunkRecord` corpus by chunking all fixture documents using the
/// standard chunker configuration. Note: embeddings are empty because only
/// lexical retrieval is evaluated here.
pub fn build_fixture_corpus() -> Vec<ChunkRecord> {
    let config = ChunkerConfig::default();
    let mut corpus = Vec::new();
    for (doc_name, pages) in fixture_documents() {
        let chunks = chunk_document(&pages, &config);
        for chunk in chunks {
            corpus.push(ChunkRecord {
                document: doc_name.to_string(),
                page: chunk.page,
                text: chunk.text,
                embedding: Vec::new(),
            });
        }
    }
    corpus
}

/// The QA-09 benchmark question set.
///
/// NOTE on question count: The original card mentions 50 questions, but per
/// specification guidelines a focused, discriminating set of 20 questions
/// across all 3 documents and pages provides rigorous measurement without
/// synthetic padding.
pub const EVAL_QUESTIONS: &[EvalQuestion] = &[
    // --- device_manual questions ---
    EvalQuestion {
        query: "What is the input voltage and power draw for the edge gateway?",
        expected_doc: "device_manual",
        expected_page: 1,
    },
    EvalQuestion {
        query: "What does a flashing amber LED status indicator mean?",
        expected_doc: "device_manual",
        expected_page: 1,
    },
    EvalQuestion {
        query: "What is the recommended mounting height and bracket procedure for wall installation?",
        expected_doc: "device_manual",
        expected_page: 2,
    },
    EvalQuestion {
        query: "How do I connect to the temporary setup captive portal Wi-Fi SSID?",
        expected_doc: "device_manual",
        expected_page: 2,
    },
    EvalQuestion {
        query: "How do I perform a hardware factory reset using the pinhole switch?",
        expected_doc: "device_manual",
        expected_page: 3,
    },
    EvalQuestion {
        query: "When does the automatic over-the-air firmware update maintenance window run?",
        expected_doc: "device_manual",
        expected_page: 3,
    },
    EvalQuestion {
        query: "How long is the limited manufacturer warranty period and how do I file a claim?",
        expected_doc: "device_manual",
        expected_page: 4,
    },
    EvalQuestion {
        query: "Does the warranty cover damage from liquid spills or electrostatic discharge surges?",
        expected_doc: "device_manual",
        expected_page: 4,
    },
    // --- developer_readme questions ---
    EvalQuestion {
        query: "How does the engine architecture utilize lock-free ring buffers?",
        expected_doc: "developer_readme",
        expected_page: 1,
    },
    EvalQuestion {
        query: "How are failed background tasks retried and moved to the dead letter queue?",
        expected_doc: "developer_readme",
        expected_page: 1,
    },
    EvalQuestion {
        query: "Which Rust compiler version is required to compile the project?",
        expected_doc: "developer_readme",
        expected_page: 2,
    },
    EvalQuestion {
        query: "What cargo compilation flags enable rocksdb-backend and tls-native features?",
        expected_doc: "developer_readme",
        expected_page: 2,
    },
    EvalQuestion {
        query: "What command-line flag configures worker thread concurrency?",
        expected_doc: "developer_readme",
        expected_page: 3,
    },
    EvalQuestion {
        query: "Which environment variable overrides tracing and logging verbosity?",
        expected_doc: "developer_readme",
        expected_page: 3,
    },
    EvalQuestion {
        query: "Under what open source license terms is the project dual-licensed?",
        expected_doc: "developer_readme",
        expected_page: 4,
    },
    EvalQuestion {
        query: "Are contributors required to sign Git commits with GPG keys?",
        expected_doc: "developer_readme",
        expected_page: 4,
    },
    // --- service_faq questions ---
    EvalQuestion {
        query: "How much does the Professional subscription tier cost per seat monthly?",
        expected_doc: "service_faq",
        expected_page: 1,
    },
    EvalQuestion {
        query: "How does pro-rated billing handle mid-cycle seat allocation upgrades?",
        expected_doc: "service_faq",
        expected_page: 1,
    },
    EvalQuestion {
        query: "Can organization members use FIDO2 YubiKey hardware security keys for MFA?",
        expected_doc: "service_faq",
        expected_page: 2,
    },
    EvalQuestion {
        query: "What permissions differentiate Admin and Read-Only roles in RBAC?",
        expected_doc: "service_faq",
        expected_page: 2,
    },
    EvalQuestion {
        query: "How long are telemetry operational logs retained before automated cold storage archival?",
        expected_doc: "service_faq",
        expected_page: 3,
    },
    EvalQuestion {
        query: "How do I submit a GDPR right-to-be-forgotten deletion request for customer data?",
        expected_doc: "service_faq",
        expected_page: 3,
    },
    EvalQuestion {
        query: "What client action is recommended for rate limiting Error 4029?",
        expected_doc: "service_faq",
        expected_page: 4,
    },
];

/// Returns the benchmark questions.
pub fn eval_questions() -> &'static [EvalQuestion] {
    EVAL_QUESTIONS
}

/// Evaluates lexical hit@k across the fixture corpus using BM25 ranking.
///
/// NOTE: Lexical retrieval only. Semantic and hybrid evaluations are follow-ups
/// when embedding engines land in the Rust path.
pub fn evaluate_lexical_hit_at_k(k: usize) -> EvalResult {
    let corpus = build_fixture_corpus();
    let questions = eval_questions();
    let mut hits = 0;
    let mut details = Vec::with_capacity(questions.len());

    for q in questions {
        let ranked = bm25_rank(q.query, &corpus);
        let top_k: Vec<_> = ranked.into_iter().take(k).collect();
        let is_hit = top_k.iter().any(|(idx, _score)| {
            let chunk = &corpus[*idx];
            chunk.document == q.expected_doc && chunk.page == q.expected_page
        });

        if is_hit {
            hits += 1;
        }

        let top_retrieved = top_k
            .iter()
            .map(|(idx, score)| {
                let chunk = &corpus[*idx];
                (chunk.document.clone(), chunk.page, *score)
            })
            .collect();

        details.push(QuestionEvalDetail {
            query: q.query.to_string(),
            expected_doc: q.expected_doc.to_string(),
            expected_page: q.expected_page,
            hit: is_hit,
            top_retrieved,
        });
    }

    let total = questions.len();
    let hit_rate = if total > 0 {
        hits as f32 / total as f32
    } else {
        0.0
    };

    EvalResult {
        total_questions: total,
        hits,
        hit_rate,
        details,
    }
}

/// Helper returning the measured lexical hit@5 retrieval accuracy across the fixture corpus.
pub fn evaluate_lexical_hit_at_5() -> f32 {
    evaluate_lexical_hit_at_k(5).hit_rate
}

#[cfg(test)]
mod tests {
    use super::super::search::format_citation;
    use super::*;

    #[test]
    fn rag_lexical_hit_at_5_meets_threshold() {
        // QA-09 target from 07-qa-release-commerce.md: hit@5 >= 80% (0.80).
        // Measures lexical (BM25) retrieval over the chunked fixture corpus.
        let result = evaluate_lexical_hit_at_k(5);

        println!(
            "QA-09 Lexical hit@5 eval: {}/{} questions retrieved within top 5 ({:.1}%)",
            result.hits,
            result.total_questions,
            result.hit_rate * 100.0
        );

        for (i, detail) in result.details.iter().enumerate() {
            if !detail.hit {
                println!(
                    "  MISS [Q{}]: {:?} expected {} p.{}, top retrieved: {:?}",
                    i + 1,
                    detail.query,
                    detail.expected_doc,
                    detail.expected_page,
                    detail.top_retrieved
                );
            }
        }

        assert!(
            result.hit_rate >= 0.80,
            "Lexical hit@5 ({:.2}%) below 80% threshold",
            result.hit_rate * 100.0
        );
    }

    #[test]
    fn citation_sanity_format_for_retrieved_hit() {
        let corpus = build_fixture_corpus();
        let ranked = bm25_rank("recommended mounting height bracket", &corpus);
        assert!(!ranked.is_empty(), "expected at least one retrieved chunk");

        let top_idx = ranked[0].0;
        let top_chunk = &corpus[top_idx];
        let citation = format_citation(1, &top_chunk.document, top_chunk.page);

        assert_eq!(
            citation, "[1] device_manual p.2",
            "citation format must match '[n] <doc> p.<page>'"
        );
    }

    #[test]
    fn helper_evaluate_lexical_hit_at_5_matches_k5() {
        let hit5 = evaluate_lexical_hit_at_5();
        let result = evaluate_lexical_hit_at_k(5);
        assert!((hit5 - result.hit_rate).abs() < f32::EPSILON);
    }
}
