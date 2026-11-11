import { positionRpc } from './position_rsocket';

/** Send an application API request to Position Management over RSocket. */
export function positionFetch(
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
  return positionRpc(
    init.method ?? 'GET',
    `${url.pathname}${url.search}`,
    body,
  );
}
