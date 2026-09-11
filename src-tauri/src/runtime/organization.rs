use std::{collections::HashMap, fmt};

use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Seniority {
    Intern,
    Junior,
    Associate,
    Mid,
    Senior,
    Staff,
    Principal,
    Director,
}

impl Seniority {
    pub const ALL: [Self; 8] = [
        Self::Intern,
        Self::Junior,
        Self::Associate,
        Self::Mid,
        Self::Senior,
        Self::Staff,
        Self::Principal,
        Self::Director,
    ];

    pub const fn level(self) -> u8 {
        match self {
            Self::Intern => 0,
            Self::Junior => 1,
            Self::Associate => 2,
            Self::Mid => 3,
            Self::Senior => 4,
            Self::Staff => 5,
            Self::Principal => 6,
            Self::Director => 7,
        }
    }

    pub const fn title_prefix(self) -> &'static str {
        match self {
            Self::Intern => "Intern",
            Self::Junior => "Junior",
            Self::Associate => "Associate",
            Self::Mid => "Mid-Level",
            Self::Senior => "Senior",
            Self::Staff => "Staff",
            Self::Principal => "Principal",
            Self::Director => "Director",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Department {
    Leadership,
    Product,
    Engineering,
    Architecture,
    DataAi,
    Quality,
    Operations,
    Security,
    Documentation,
    Research,
}

impl Department {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Leadership => "Leadership",
            Self::Product => "Product",
            Self::Engineering => "Engineering",
            Self::Architecture => "Architecture",
            Self::DataAi => "Data & AI",
            Self::Quality => "Quality",
            Self::Operations => "Operations",
            Self::Security => "Security",
            Self::Documentation => "Documentation",
            Self::Research => "Research",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentFunction {
    Director,
    EngineeringManager,
    ProductManager,
    TechnicalProductManager,
    TechnicalProgramManager,
    ProjectCoordinator,
    BusinessAnalyst,
    ProductAnalyst,
    SoftwareEngineer,
    BackendEngineering,
    FrontendEngineering,
    FullStackEngineering,
    MobileEngineering,
    AutomationEngineering,
    PlatformEngineering,
    DevopsEngineering,
    DatabaseEngineering,
    DataEngineering,
    DataScience,
    MlEngineering,
    AiEngineering,
    LlmEngineering,
    ResearchAnalyst,
    SoftwareArchitecture,
    SolutionArchitecture,
    AiArchitecture,
    QaEngineering,
    TestEngineering,
    Reviewer,
    SecurityEngineering,
    TechnicalWriting,
    GenericSoftwareAgent,
}

impl AgentFunction {
    pub const fn base_title(self) -> &'static str {
        match self {
            Self::Director => "Director",
            Self::EngineeringManager => "Engineering Manager",
            Self::ProductManager => "Product Manager",
            Self::TechnicalProductManager => "Technical Product Manager",
            Self::TechnicalProgramManager => "Technical Program Manager",
            Self::ProjectCoordinator => "Project Coordinator",
            Self::BusinessAnalyst => "Business Analyst",
            Self::ProductAnalyst => "Product Analyst",
            Self::SoftwareEngineer | Self::GenericSoftwareAgent => "Software Engineer",
            Self::BackendEngineering => "Backend Engineer",
            Self::FrontendEngineering => "Frontend Engineer",
            Self::FullStackEngineering => "Full-Stack Engineer",
            Self::MobileEngineering => "Mobile Engineer",
            Self::AutomationEngineering => "Automation Engineer",
            Self::PlatformEngineering => "Platform Engineer",
            Self::DevopsEngineering => "DevOps Engineer",
            Self::DatabaseEngineering => "Database Engineer",
            Self::DataEngineering => "Data Engineer",
            Self::DataScience => "Data Scientist",
            Self::MlEngineering => "ML Engineer",
            Self::AiEngineering => "AI Engineer",
            Self::LlmEngineering => "LLM Engineer",
            Self::ResearchAnalyst => "Research Analyst",
            Self::SoftwareArchitecture => "Software Architect",
            Self::SolutionArchitecture => "Solution Architect",
            Self::AiArchitecture => "AI Architect",
            Self::QaEngineering => "QA Engineer",
            Self::TestEngineering => "Test Engineer",
            Self::Reviewer => "Reviewer",
            Self::SecurityEngineering => "Security Engineer",
            Self::TechnicalWriting => "Technical Writer",
        }
    }

    pub const fn department(self) -> Department {
        match self {
            Self::Director | Self::EngineeringManager => Department::Leadership,
            Self::ProductManager
            | Self::TechnicalProductManager
            | Self::TechnicalProgramManager
            | Self::ProjectCoordinator
            | Self::BusinessAnalyst
            | Self::ProductAnalyst => Department::Product,
            Self::SoftwareEngineer
            | Self::BackendEngineering
            | Self::FrontendEngineering
            | Self::FullStackEngineering
            | Self::MobileEngineering
            | Self::AutomationEngineering
            | Self::GenericSoftwareAgent => Department::Engineering,
            Self::PlatformEngineering | Self::DevopsEngineering | Self::DatabaseEngineering => {
                Department::Operations
            }
            Self::DataEngineering
            | Self::DataScience
            | Self::MlEngineering
            | Self::AiEngineering
            | Self::LlmEngineering => Department::DataAi,
            Self::ResearchAnalyst => Department::Research,
            Self::SoftwareArchitecture | Self::SolutionArchitecture | Self::AiArchitecture => {
                Department::Architecture
            }
            Self::QaEngineering | Self::TestEngineering | Self::Reviewer => Department::Quality,
            Self::SecurityEngineering => Department::Security,
            Self::TechnicalWriting => Department::Documentation,
        }
    }

    pub fn supports(self, seniority: Seniority) -> bool {
        let level = seniority.level();
        match self {
            Self::Director => seniority == Seniority::Director,
            Self::EngineeringManager => (4..=7).contains(&level),
            Self::ProductManager
            | Self::TechnicalProductManager
            | Self::TechnicalProgramManager => (2..=7).contains(&level),
            Self::ProjectCoordinator | Self::BusinessAnalyst | Self::ProductAnalyst => {
                (1..=6).contains(&level)
            }
            Self::SoftwareArchitecture | Self::SolutionArchitecture | Self::AiArchitecture => {
                (4..=6).contains(&level)
            }
            Self::Reviewer => (2..=6).contains(&level),
            _ => level <= 6,
        }
    }

    pub fn is_coding(self) -> bool {
        matches!(
            self,
            Self::SoftwareEngineer
                | Self::BackendEngineering
                | Self::FrontendEngineering
                | Self::FullStackEngineering
                | Self::MobileEngineering
                | Self::AutomationEngineering
                | Self::PlatformEngineering
                | Self::DevopsEngineering
                | Self::DatabaseEngineering
                | Self::DataEngineering
                | Self::MlEngineering
                | Self::AiEngineering
                | Self::LlmEngineering
                | Self::QaEngineering
                | Self::TestEngineering
                | Self::SecurityEngineering
                | Self::GenericSoftwareAgent
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CapabilityScore(u8);

impl CapabilityScore {
    pub fn new(value: u8) -> Result<Self, String> {
        (value <= 100)
            .then_some(Self(value))
            .ok_or_else(|| format!("capability score must be between 0 and 100, got {value}"))
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

impl Serialize for CapabilityScore {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(self.0)
    }
}

impl<'de> Deserialize<'de> for CapabilityScore {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u8::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CapabilityProfile {
    pub coding: Option<CapabilityScore>,
    pub debugging: Option<CapabilityScore>,
    pub planning: Option<CapabilityScore>,
    pub architecture: Option<CapabilityScore>,
    pub tool_use: Option<CapabilityScore>,
    pub instruction_following: Option<CapabilityScore>,
    pub long_context: Option<CapabilityScore>,
    pub test_generation: Option<CapabilityScore>,
    pub review: Option<CapabilityScore>,
    pub research: Option<CapabilityScore>,
    pub speed: Option<CapabilityScore>,
    pub reliability: Option<CapabilityScore>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelationshipType {
    Reporting,
    Collaboration,
    Dependency,
    Handoff,
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationRelationship {
    pub id: String,
    #[serde(rename = "type")]
    pub relationship_type: RelationshipType,
    pub source: String,
    pub target: String,
    pub persistent: bool,
    pub task_id: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecommendationPolicy {
    pub intern_min: u8,
    pub junior_min: u8,
    pub associate_min: u8,
    pub mid_min: u8,
    pub senior_min: u8,
}

impl Default for RecommendationPolicy {
    fn default() -> Self {
        Self {
            intern_min: 40,
            junior_min: 55,
            associate_min: 70,
            mid_min: 75,
            senior_min: 80,
        }
    }
}

impl RecommendationPolicy {
    pub fn recommend(
        &self,
        function: AgentFunction,
        capabilities: &CapabilityProfile,
    ) -> Option<Seniority> {
        let score = role_score(function, capabilities)?.get();
        let candidate = if score >= self.senior_min {
            Seniority::Senior
        } else if score >= self.mid_min {
            Seniority::Mid
        } else if score >= self.associate_min {
            Seniority::Associate
        } else if score >= self.junior_min {
            Seniority::Junior
        } else if score >= self.intern_min {
            Seniority::Intern
        } else {
            return None;
        };
        Seniority::ALL
            .iter()
            .copied()
            .filter(|level| level.level() <= candidate.level() && function.supports(*level))
            .max_by_key(|level| level.level())
    }
}

fn role_score(
    function: AgentFunction,
    capabilities: &CapabilityProfile,
) -> Option<CapabilityScore> {
    match function.department() {
        Department::Product | Department::Leadership => capabilities
            .planning
            .or(capabilities.instruction_following)
            .or(capabilities.research),
        Department::Architecture => capabilities
            .architecture
            .or(capabilities.planning)
            .or(capabilities.coding),
        Department::Quality => capabilities
            .review
            .or(capabilities.test_generation)
            .or(capabilities.debugging),
        Department::Research => capabilities.research.or(capabilities.long_context),
        _ => capabilities
            .coding
            .or(capabilities.debugging)
            .or(capabilities.tool_use),
    }
}

pub fn legacy_identity(role: &str) -> (Option<Seniority>, AgentFunction) {
    let normalized = role
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    let seniority = if normalized.contains("director") {
        Some(Seniority::Director)
    } else if normalized.contains("principal") {
        Some(Seniority::Principal)
    } else if normalized.contains("staff") {
        Some(Seniority::Staff)
    } else if normalized.contains("senior") {
        Some(Seniority::Senior)
    } else if normalized.contains("midlevel") || normalized.starts_with("mid") {
        Some(Seniority::Mid)
    } else if normalized.contains("associate") {
        Some(Seniority::Associate)
    } else if normalized.contains("junior") {
        Some(Seniority::Junior)
    } else if normalized.contains("intern") {
        Some(Seniority::Intern)
    } else {
        None
    };
    let function = if normalized.contains("director") {
        AgentFunction::Director
    } else if normalized.contains("engineeringmanager") {
        AgentFunction::EngineeringManager
    } else if normalized.contains("technicalproductmanager") {
        AgentFunction::TechnicalProductManager
    } else if normalized.contains("technicalprogrammanager") {
        AgentFunction::TechnicalProgramManager
    } else if normalized.contains("productmanager") {
        AgentFunction::ProductManager
    } else if normalized.contains("projectcoordinator") {
        AgentFunction::ProjectCoordinator
    } else if normalized.contains("productanalyst") {
        AgentFunction::ProductAnalyst
    } else if normalized.contains("businessanalyst") {
        AgentFunction::BusinessAnalyst
    } else if normalized.contains("backend") {
        AgentFunction::BackendEngineering
    } else if normalized.contains("frontend") {
        AgentFunction::FrontendEngineering
    } else if normalized.contains("fullstack") {
        AgentFunction::FullStackEngineering
    } else if normalized.contains("mobile") {
        AgentFunction::MobileEngineering
    } else if normalized.contains("automation") {
        AgentFunction::AutomationEngineering
    } else if normalized.contains("platform") {
        AgentFunction::PlatformEngineering
    } else if normalized.contains("devops") {
        AgentFunction::DevopsEngineering
    } else if normalized.contains("database") {
        AgentFunction::DatabaseEngineering
    } else if normalized.contains("dataengineer") {
        AgentFunction::DataEngineering
    } else if normalized.contains("datascientist") {
        AgentFunction::DataScience
    } else if normalized.contains("researchanalyst") {
        AgentFunction::ResearchAnalyst
    } else if normalized.contains("llm") {
        AgentFunction::LlmEngineering
    } else if normalized.contains("mlengineer") {
        AgentFunction::MlEngineering
    } else if normalized.contains("aiengineer") {
        AgentFunction::AiEngineering
    } else if normalized.contains("solutionarchitect") {
        AgentFunction::SolutionArchitecture
    } else if normalized.contains("aiarchitect") {
        AgentFunction::AiArchitecture
    } else if normalized.contains("softwarearchitect") || normalized.contains("architect") {
        AgentFunction::SoftwareArchitecture
    } else if normalized.contains("security") {
        AgentFunction::SecurityEngineering
    } else if normalized.contains("review") {
        AgentFunction::Reviewer
    } else if normalized.contains("qa") {
        AgentFunction::QaEngineering
    } else if normalized.contains("test") {
        AgentFunction::TestEngineering
    } else if normalized.contains("writer") {
        AgentFunction::TechnicalWriting
    } else if normalized.contains("softwareengineer") || normalized.contains("developer") {
        AgentFunction::SoftwareEngineer
    } else {
        AgentFunction::GenericSoftwareAgent
    };
    (seniority, function)
}

pub fn display_title(seniority: Option<Seniority>, function: AgentFunction) -> String {
    if function == AgentFunction::Director {
        return "Director".into();
    }
    seniority.map_or_else(
        || function.base_title().into(),
        |level| format!("{} {}", level.title_prefix(), function.base_title()),
    )
}

pub fn hierarchy_warnings(parents: &HashMap<String, Option<String>>) -> Vec<String> {
    let mut warnings = Vec::new();
    for (agent, parent) in parents {
        if let Some(parent) = parent {
            if parent != "god" && !parents.contains_key(parent) {
                warnings.push(format!("ORPHAN:{agent}:{parent}"));
            }
        }
        let mut visited = vec![agent.as_str()];
        let mut current = parent.as_deref();
        while let Some(node) = current {
            if node == "god" {
                break;
            }
            if visited.contains(&node) {
                warnings.push(format!("CYCLE:{}", visited.join("->")));
                break;
            }
            visited.push(node);
            current = parents.get(node).and_then(Option::as_deref);
        }
    }
    warnings.sort();
    warnings.dedup();
    warnings
}

impl fmt::Display for Department {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seniority_serialization_has_one_source_of_truth_for_level() {
        let json = serde_json::to_string(&Seniority::Senior).unwrap();
        assert_eq!(json, "\"SENIOR\"");
        assert_eq!(Seniority::Senior.level(), 4);
        assert!(serde_json::from_str::<Seniority>("\"GOD\"").is_err());
    }

    #[test]
    fn capability_scores_reject_out_of_range_values() {
        assert!(serde_json::from_str::<CapabilityProfile>(r#"{"coding":101}"#).is_err());
        assert_eq!(CapabilityScore::new(80).unwrap().get(), 80);
        let profile = CapabilityProfile {
            coding: CapabilityScore::new(88).ok(),
            reliability: CapabilityScore::new(91).ok(),
            ..CapabilityProfile::default()
        };
        assert_eq!(
            serde_json::from_str::<CapabilityProfile>(&serde_json::to_string(&profile).unwrap())
                .unwrap(),
            profile
        );
    }

    #[test]
    fn titles_and_legacy_roles_are_typed() {
        let (level, function) = legacy_identity("SeniorBackendEngineer");
        assert_eq!(level, Some(Seniority::Senior));
        assert_eq!(function, AgentFunction::BackendEngineering);
        assert_eq!(display_title(level, function), "Senior Backend Engineer");
        assert!(!AgentFunction::ProductManager.supports(Seniority::Intern));
    }

    #[test]
    fn recommendation_is_function_specific_and_unknown_stays_unknown() {
        let profile = CapabilityProfile {
            coding: CapabilityScore::new(60).ok(),
            planning: CapabilityScore::new(86).ok(),
            ..CapabilityProfile::default()
        };
        let policy = RecommendationPolicy::default();
        assert_eq!(
            policy.recommend(AgentFunction::SoftwareEngineer, &profile),
            Some(Seniority::Junior)
        );
        assert_eq!(
            policy.recommend(AgentFunction::ProductAnalyst, &profile),
            Some(Seniority::Senior)
        );
        assert_eq!(
            policy.recommend(
                AgentFunction::SoftwareEngineer,
                &CapabilityProfile::default()
            ),
            None
        );
    }

    #[test]
    fn hierarchy_reports_orphans_and_cycles_without_recursing_forever() {
        let parents = HashMap::from([
            ("a".into(), Some("b".into())),
            ("b".into(), Some("a".into())),
            ("orphan".into(), Some("missing".into())),
        ]);
        let warnings = hierarchy_warnings(&parents);
        assert!(warnings.iter().any(|warning| warning.starts_with("CYCLE:")));
        assert!(warnings
            .iter()
            .any(|warning| warning.starts_with("ORPHAN:")));
    }
}
