export const SENIORITIES = [
  ['INTERN', 0], ['JUNIOR', 1], ['ASSOCIATE', 2], ['MID', 3],
  ['SENIOR', 4], ['STAFF', 5], ['PRINCIPAL', 6], ['DIRECTOR', 7]
];

export const FUNCTIONS = {
  DIRECTOR: {title:'Director', department:'LEADERSHIP', levels:[7]},
  ENGINEERING_MANAGER: {title:'Engineering Manager', department:'LEADERSHIP', levels:[4,5,6]},
  PRODUCT_MANAGER: {title:'Product Manager', department:'PRODUCT', levels:[2,3,4,5,6]},
  TECHNICAL_PRODUCT_MANAGER: {title:'Technical Product Manager', department:'PRODUCT', levels:[2,3,4,5,6]},
  TECHNICAL_PROGRAM_MANAGER: {title:'Technical Program Manager', department:'PRODUCT', levels:[2,3,4,5,6]},
  PROJECT_COORDINATOR: {title:'Project Coordinator', department:'PRODUCT', levels:[1,2,3,4,5,6]},
  BUSINESS_ANALYST: {title:'Business Analyst', department:'PRODUCT', levels:[1,2,3,4,5,6]},
  PRODUCT_ANALYST: {title:'Product Analyst', department:'PRODUCT', levels:[1,2,3,4,5,6]},
  SOFTWARE_ENGINEER: {title:'Software Engineer', department:'ENGINEERING', levels:[0,1,2,3,4,5,6]},
  BACKEND_ENGINEERING: {title:'Backend Engineer', department:'ENGINEERING', levels:[0,1,2,3,4,5,6]},
  FRONTEND_ENGINEERING: {title:'Frontend Engineer', department:'ENGINEERING', levels:[0,1,2,3,4,5,6]},
  FULL_STACK_ENGINEERING: {title:'Full-Stack Engineer', department:'ENGINEERING', levels:[0,1,2,3,4,5,6]},
  MOBILE_ENGINEERING: {title:'Mobile Engineer', department:'ENGINEERING', levels:[0,1,2,3,4,5,6]},
  AUTOMATION_ENGINEERING: {title:'Automation Engineer', department:'ENGINEERING', levels:[0,1,2,3,4,5,6]},
  PLATFORM_ENGINEERING: {title:'Platform Engineer', department:'OPERATIONS', levels:[0,1,2,3,4,5,6]},
  DEVOPS_ENGINEERING: {title:'DevOps Engineer', department:'OPERATIONS', levels:[0,1,2,3,4,5,6]},
  DATABASE_ENGINEERING: {title:'Database Engineer', department:'OPERATIONS', levels:[0,1,2,3,4,5,6]},
  DATA_ENGINEERING: {title:'Data Engineer', department:'DATA_AI', levels:[0,1,2,3,4,5,6]},
  DATA_SCIENCE: {title:'Data Scientist', department:'DATA_AI', levels:[0,1,2,3,4,5,6]},
  ML_ENGINEERING: {title:'ML Engineer', department:'DATA_AI', levels:[0,1,2,3,4,5,6]},
  AI_ENGINEERING: {title:'AI Engineer', department:'DATA_AI', levels:[0,1,2,3,4,5,6]},
  LLM_ENGINEERING: {title:'LLM Engineer', department:'DATA_AI', levels:[0,1,2,3,4,5,6]},
  SOFTWARE_ARCHITECTURE: {title:'Software Architect', department:'ARCHITECTURE', levels:[4,5,6]},
  SOLUTION_ARCHITECTURE: {title:'Solution Architect', department:'ARCHITECTURE', levels:[4,5,6]},
  AI_ARCHITECTURE: {title:'AI Architect', department:'ARCHITECTURE', levels:[4,5,6]},
  QA_ENGINEERING: {title:'QA Engineer', department:'QUALITY', levels:[0,1,2,3,4,5,6]},
  TEST_ENGINEERING: {title:'Test Engineer', department:'QUALITY', levels:[0,1,2,3,4,5,6]},
  REVIEWER: {title:'Reviewer', department:'QUALITY', levels:[2,3,4,5,6]},
  SECURITY_ENGINEERING: {title:'Security Engineer', department:'SECURITY', levels:[0,1,2,3,4,5,6]},
  TECHNICAL_WRITING: {title:'Technical Writer', department:'DOCUMENTATION', levels:[0,1,2,3,4,5,6]},
  RESEARCH_ANALYST: {title:'Research Analyst', department:'RESEARCH', levels:[0,1,2,3,4,5,6]}
};

export function validFunctionsForSeniority(seniority) {
  const level = SENIORITIES.find(([name]) => name === seniority)?.[1];
  return Object.entries(FUNCTIONS).filter(([, value]) => value.levels.includes(level));
}

export function createMutationRequest(revision, mutation, reason = null) {
  return {
    actor:{id:'god', scope:'PROJECT'},
    expectedRevision:revision,
    taskId:null,
    reason:reason || null,
    mutation
  };
}

export function relationshipIsEditable(relationship) {
  return Boolean(relationship?.persistent)
    && ['COLLABORATION', 'REVIEW', 'ADVISORY'].includes(relationship.type);
}

export function openCount(records = []) {
  return records.filter(record => record.status === 'OPEN' || record.status === 'PENDING').length;
}

export function conflictMessage(error) {
  const message = String(error?.message ?? error ?? '');
  return message.toLowerCase().includes('revision conflict')
    ? 'Organization changed elsewhere. The latest state was loaded; review and retry your change.'
    : message;
}
