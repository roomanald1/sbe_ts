import { BehaviorSubject } from 'rxjs';

export type AutoTraderOutcome = {
  decisionId: string; symbol: string; side: string; openedAt: number; closedAt: number;
  quantity: number; entryPrice: number; pnl: number; direction: string; strength: number | null;
  scout: boolean; components: Record<string, number | null>;
};

type AuditRecord = { decision_id?: string; stage?: string; symbol?: string; timestamp?: number; side?: string; direction?: string; filled_quantity?: number; average_price?: number; exit_filled_quantity?: number; realized_pnl?: number; strength?: number; scout?: boolean; signal_components?: Record<string, number | null> };

export class AutoTraderOutcomesModel {
  readonly state$ = new BehaviorSubject<{ rows: AutoTraderOutcome[]; loading: boolean; error?: string }>({ rows: [], loading: true });
  private timer?: ReturnType<typeof setInterval>;
  async start() { if (this.timer) return; await this.refresh(); this.timer = setInterval(() => void this.refresh(), 30_000); }
  stop() { if (this.timer) clearInterval(this.timer); this.timer = undefined; }
  private async refresh() {
    try {
      const response = await fetch('/signals/auto-trader-audit?limit=50000');
      if (!response.ok) throw new Error('Audit unavailable');
      const records = await response.json() as AuditRecord[];
      const trades = new Map<string, { entry?: AuditRecord; exits: AuditRecord[] }>();
      for (const record of records) {
        if (!record.decision_id) continue;
        const trade = trades.get(record.decision_id) ?? { exits: [] };
        if (record.stage === 'entry') trade.entry = record;
        else if (record.stage?.startsWith('exit')) trade.exits.push(record);
        trades.set(record.decision_id, trade);
      }
      const rows = [...trades.entries()].flatMap(([decisionId, trade]) => {
        const entry = trade.entry;
        if (!entry || !trade.exits.length || !(entry.filled_quantity && entry.average_price)) return [];
        const exited = trade.exits.reduce((sum, row) => sum + (row.exit_filled_quantity ?? 0), 0);
        if (exited + 1e-12 < entry.filled_quantity) return [];
        return [{ decisionId, symbol: entry.symbol ?? '—', side: entry.side ?? '—', openedAt: entry.timestamp ?? 0,
          closedAt: Math.max(...trade.exits.map((row) => row.timestamp ?? 0)), quantity: entry.filled_quantity,
          entryPrice: entry.average_price, pnl: trade.exits.reduce((sum, row) => sum + (row.realized_pnl ?? 0), 0),
          direction: entry.direction ?? '—', strength: entry.strength ?? null, scout: entry.scout ?? false,
          components: entry.signal_components ?? {} }];
      }).sort((a, b) => b.closedAt - a.closedAt);
      this.state$.next({ rows, loading: false });
    } catch { this.state$.next({ ...this.state$.value, loading: false, error: 'AutoTrader closed trade audit is unavailable.' }); }
  }
}
