export class ValidationError extends Error {
  constructor(message, details = undefined) {
    super(message);
    this.name = 'ValidationError';
    this.details = details;
  }
}

export class InvalidTransitionError extends Error {
  constructor(entity, from, to) {
    super(`Invalid ${entity} state transition: ${from} -> ${to}`);
    this.name = 'InvalidTransitionError';
  }
}

export class ResourceUnavailableError extends Error {
  constructor(message, { resetAt = null, status = 'RATE_LIMITED' } = {}) {
    super(message);
    this.name = 'ResourceUnavailableError';
    this.resetAt = resetAt;
    this.status = status;
  }
}
