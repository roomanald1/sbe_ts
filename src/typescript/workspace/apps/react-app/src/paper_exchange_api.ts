import { paperExchangeRpc } from './position_rsocket';

/** Send a paper order request through the Paper Exchange RSocket service. */
export function paperExchangeFetch(
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
  return paperExchangeRpc(
    init.method ?? 'GET',
    `${url.pathname}${url.search}`,
    body,
  );
}
