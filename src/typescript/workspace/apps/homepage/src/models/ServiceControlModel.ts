import {
  Subject,
  catchError,
  concat,
  from,
  map,
  merge,
  of,
  shareReplay,
  switchMap,
  timer,
} from 'rxjs';

export type Project = 'system' | 'infra';
export type ServiceStatus = {
  kind: 'running' | 'pending' | 'stopped';
  label: string;
  detail: string;
};
export type Notice = { text: string; error: boolean };
export type ServiceSnapshot = {
  state: Record<Project, Map<string, string>>;
  updated: string;
  available: boolean;
};
export type ServiceAction = 'start' | 'stop' | 'restart';
export type ServiceTotals = {
  running: number;
  pending: number;
  stopped: number;
};

export const services: Record<Project, string[]> = {
  system: [
    'feed-handlers',
    'position-management',
    'execution',
    'insights',
    'insights-maintenance',
    'live-signals',
    'auto-trader',
    'news-sentiment',
    'learning',
    'scouts',
    'web',
  ],
  infra: [
    'prometheus',
    'loki',
    'alloy',
    'node-exporter',
    'app-exporter',
    'grafana',
  ],
};

export const swaggerPorts: Record<string, number> = {
  'feed-handlers': 3001,
  'position-management': 3002,
  execution: 3003,
  insights: 3004,
  'live-signals': 3007,
  'auto-trader': 3005,
  learning: 3010,
  scouts: 3011,
};

export const initialServiceSnapshot: ServiceSnapshot = {
  state: { system: new Map(), infra: new Map() },
  updated: 'Connecting…',
  available: true,
};
export const initialServiceTotals: ServiceTotals = {
  running: 0,
  pending: 0,
  stopped: 0,
};

export function parseServices(lines: string): Map<string, string> {
  return new Map(
    lines
      .split('\n')
      .filter(Boolean)
      .map((line) => {
        try {
          const container = JSON.parse(line) as {
            Service?: string;
            Name?: string;
            State?: string;
            Health?: string;
          };
          return [
            container.Service || container.Name || '',
            `${container.State || 'stopped'}${container.Health ? ` · ${container.Health}` : ''}`,
          ];
        } catch {
          return [line.split(/\s+/)[0], line];
        }
      }),
  );
}

export function statusFor(value?: string): ServiceStatus {
  if (!value)
    return { kind: 'stopped', label: 'Off', detail: 'Container is stopped' };
  const [state, health] = value.split(' · ');
  if (state === 'running' && (!health || health === 'healthy'))
    return {
      kind: 'running',
      label: 'On',
      detail: health === 'healthy' ? 'Running · healthy' : 'Running',
    };
  if (state === 'running')
    return {
      kind: 'pending',
      label: health === 'unhealthy' ? 'Unhealthy' : 'Starting',
      detail: `Running · ${health}`,
    };
  if (['created', 'restarting', 'starting', 'paused'].includes(state))
    return {
      kind: 'pending',
      label: 'Starting',
      detail: state[0].toUpperCase() + state.slice(1),
    };
  return {
    kind: 'stopped',
    label: 'Off',
    detail: state[0].toUpperCase() + state.slice(1),
  };
}

export class ServiceControlModel {
  private readonly refreshTrigger = new Subject<void>();
  private readonly noticeTrigger = new Subject<Notice>();

  readonly snapshot$ = merge(timer(0, 10000), this.refreshTrigger).pipe(
    switchMap(() =>
      from(this.fetchSnapshot()).pipe(
        catchError(() =>
          of({
            ...initialServiceSnapshot,
            updated: 'Status unavailable',
            available: false,
          }),
        ),
      ),
    ),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  readonly totals$ = this.snapshot$.pipe(
    map(({ state }) => this.summarize(state)),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  readonly notice$ = this.noticeTrigger.pipe(
    switchMap((notice) =>
      concat(of(notice), timer(4200).pipe(map(() => null))),
    ),
    shareReplay({ bufferSize: 1, refCount: true }),
  );

  refresh(): void {
    this.refreshTrigger.next();
  }

  async action(
    project: Project | 'all',
    verb: ServiceAction,
    service?: string,
  ): Promise<void> {
    const target = service || (project === 'all' ? 'everything' : project);
    this.showNotice({
      text: `${verb === 'start' ? 'Starting' : verb === 'stop' ? 'Stopping' : 'Restarting'} ${target}…`,
      error: false,
    });
    try {
      const response = await fetch('/api/action', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ project, action: verb, service }),
      });
      const result = (await response.json()) as { error?: string };
      if (!response.ok) throw new Error(result.error || 'Action failed');
      this.showNotice({
        text: `${service || (project === 'all' ? 'All services' : project)} ${verb === 'start' ? 'started' : verb === 'stop' ? 'stopped' : 'restarted'}`,
        error: false,
      });
      this.refresh();
    } catch (error) {
      this.showNotice({
        text: error instanceof Error ? error.message : 'Action failed',
        error: true,
      });
    }
  }

  private summarize(state: ServiceSnapshot['state']): ServiceTotals {
    const rows = (Object.keys(services) as Project[]).flatMap((project) =>
      services[project].map((name) => statusFor(state[project].get(name))),
    );
    return {
      running: rows.filter((item) => item.kind === 'running').length,
      pending: rows.filter((item) => item.kind === 'pending').length,
      stopped: rows.filter((item) => item.kind === 'stopped').length,
    };
  }

  private showNotice(notice: Notice): void {
    this.noticeTrigger.next(notice);
  }

  private async fetchSnapshot(): Promise<ServiceSnapshot> {
    const response = await fetch('/api/status');
    if (!response.ok) throw new Error('Could not read service status');
    const result = (await response.json()) as Record<Project, string>;
    return {
      state: {
        system: parseServices(result.system),
        infra: parseServices(result.infra),
      },
      updated: `Updated ${new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}`,
      available: true,
    };
  }
}
