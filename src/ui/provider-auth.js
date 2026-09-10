const LOCAL_PROVIDERS = new Set(['mock', 'ollama']);

export function authModeForProvider(provider) {
  return LOCAL_PROVIDERS.has(provider) ? 'local' : 'subscription';
}
