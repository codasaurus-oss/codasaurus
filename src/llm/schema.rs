//! Structured review output: the JSON schema sent to the gateway and the types
//! the model's reply deserializes into.

use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmReviewOutput {
    pub verdict: String,
    pub issues: Vec<LlmIssue>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmIssue {
    pub severity: String,
    pub category: String,
    pub file: String,
    #[serde(default)]
    pub line: usize,
    pub description: String,
    pub suggestion: Option<String>,
    #[serde(default = "default_confidence")]
    pub confidence: String,
    /// Why the model believes this (citation / rationale). Optional for older models.
    #[serde(default)]
    pub rationale: Option<String>,
    /// Structured one-click fix: `original` must appear verbatim in the file for
    /// this to survive re-verification (see `bot::provenance::reverify_llm_issues`).
    #[serde(default)]
    pub replacement: Option<LlmReplacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmReplacement {
    pub original: String,
    pub replacement: String,
}

fn default_confidence() -> String {
    "medium".to_string()
}

pub fn review_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "verdict": {
                "type": "string",
                "enum": ["ship", "fix-before-ship", "hold"],
                "description": "Overall verdict: ship | fix-before-ship | hold"
            },
            "issues": {
                "type": "array",
                "maxItems": 8,
                "items": {
                    "type": "object",
                    "properties": {
                        "severity": {
                            "type": "string",
                            "enum": ["critical", "warning", "info"],
                            "description": "critical = merge-blocking production risk; warning = should fix; info = optional"
                        },
                        "category": {
                            "type": "string",
                            "enum": ["security", "logic", "correctness", "performance", "maintainability"],
                            "description": "Category of the issue"
                        },
                        "file": {
                            "type": "string",
                            "description": "File path from the diff"
                        },
                        "line": {
                            "type": "integer",
                            "description": "Line number on the new (+) side of the diff"
                        },
                        "description": {
                            "type": "string",
                            "description": "What is wrong + why it matters in production (1-2 sentences)"
                        },
                        "suggestion": {
                            "type": "string",
                            "description": "Concrete fix (<=40 words), ideally a short code hint"
                        },
                        "confidence": {
                            "type": "string",
                            "enum": ["high", "medium", "low"],
                            "description": "high = clear evidence in diff; medium = likely; low = speculative"
                        },
                        "rationale": {
                            "type": "string",
                            "description": "Evidence citing specific symbols/lines from the diff"
                        },
                        "replacement": {
                            "type": ["object", "null"],
                            "description": "Optional one-click fix. `original` MUST be copied verbatim (exact whitespace) from the diff's + side; only set this when you are certain of the exact existing text.",
                            "properties": {
                                "original": {
                                    "type": "string",
                                    "description": "Exact existing line(s) to replace, copied verbatim from the diff"
                                },
                                "replacement": {
                                    "type": "string",
                                    "description": "The replacement line(s)"
                                }
                            },
                            "required": ["original", "replacement"],
                            "additionalProperties": false
                        }
                    },
                    "required": [
                        "severity",
                        "category",
                        "file",
                        "line",
                        "description",
                        "suggestion",
                        "confidence",
                        "rationale"
                    ],
                    "additionalProperties": false
                },
                "description": "High-confidence issues only; empty array is valid and preferred over weak findings"
            },
            "summary": {
                "type": "string",
                "description": "Optional 1-2 sentence overall assessment"
            }
        },
        "required": ["verdict", "issues"],
        "additionalProperties": false
    })
}

/// `response_format` for structured reviews. The gateway forwards it verbatim,
/// so a provider that supports strict JSON schema enforces the shape.
pub(crate) fn review_response_format() -> serde_json::Value {
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "review_response",
            "strict": true,
            "schema": review_schema()
        }
    })
}

/// LLM judge verdict for one finding: 0-5 confidence + rationale.
#[derive(Debug, Clone)]
pub struct JudgeOutcome {
    pub index: usize,
    pub confidence: u8,
    pub rationale: String,
}
