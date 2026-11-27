import { paperExchangeRpc, positionRpc } from './position_rsocket';

const pendingGets = new Map<string, Promise<Response>>();
const cachedGets = new Map<
  string,
  { body: string; expiresAt: number; headers: [string, string][] }
>();

/** Share concurrent idempotent reads across models; each caller gets its own body. */
export function sharedGet(url: string, cacheForMs = 0): Promise<Response> {
  const cached = cachedGets.get(url);
  if (cached && cached.expiresAt > Date.now()) {
    return Promise.resolve(
      new Response(cached.body, { status: 200, headers: cached.headers }),
    );
  }
  if (cached) cachedGets.delete(url);

  let pending = pendingGets.get(url);
  if (!pending) {
    pending = (
      usesExecutionRSocket(url)
        ? paperExchangeRpc('GET', url)
        : usesPositionManagementRSocket(url)
          ? positionRpc('GET', url)
          : fetch(url)
    )
      .then(async (response) => {
        if (
          cacheForMs > 0 &&
          response.ok &&
          typeof response.clone === 'function'
        ) {
          cachedGets.set(url, {
            body: await response.clone().text(),
            expiresAt: Date.now() + cacheForMs,
            headers: (() => {
              const headers: [string, string][] = [];
              response.headers.forEach((value, name) => {
                headers.push([name, value]);
              });
              return headers;
            })(),
          });
        }
        return response;
      })
      .finally(() => pendingGets.delete(url));
    pendingGets.set(url, pending);
  }
  return pending.then((response) =>
    typeof response.clone === 'function' ? response.clone() : response,
  );
}
export function usesExecutionRSocket(url: string): boolean {
  const path = new URL(
    url,
    typeof window === 'undefined' ? 'http://localhost' : window.location.href,
  ).pathname;
  return (
    path.startsWith('/positions/orders') ||
    path.startsWith('/positions/vwap') ||
    path.startsWith('/positions/twap') ||
    path.startsWith('/positions/barriers') ||
    path.startsWith('/positions/close-profitable')
  );
}

export function usesPositionManagementRSocket(url: string): boolean {
  const path = new URL(
    url,
    typeof window === 'undefined' ? 'http://localhost' : window.location.href,
  ).pathname;
  return (
    path === '/positions' ||
    path.startsWith('/positions/operational') ||
    path.startsWith('/positions/trades') ||
    path.startsWith('/positions/account') ||
    path.startsWith('/positions/realized-pnl') ||
    path.startsWith('/positions/fee-') ||
    path.startsWith('/positions/usd-rates') ||
    path === '/account' ||
    path.startsWith('/users/')
  );
}
