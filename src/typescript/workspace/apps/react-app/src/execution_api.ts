import { executionRpc } from './position_rsocket';

/** Send an order execution API request over the Execution RSocket service. */
export function executionFetch(
  input: string,
  init: RequestInit = {},
): Promise<Response> {
  const url = new URL(
    input,
    typeof window === 'undefined' ? 'http://localhost' : window.location.href,
  );
  let body: unknown;
  if (init.body !== undefined && init.body !== null) {
    const raw = typeof init.body === 'string' ? init.body : String(init.body);
    try {
      body = JSON.parse(raw) as unknown;
    } catch {
      body = raw;
    }
  }
  return executionRpc(
    init.method ?? 'GET',
    `${url.pathname}${url.search}`,
    body,
  );
}
