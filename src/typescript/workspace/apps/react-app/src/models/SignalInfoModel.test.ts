import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { SignalInfoModel } from './SignalInfoModel';

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  onopen?: () => void;
  onmessage?: (event: { data: string }) => void;
  onclose?: () => void;
  onerror?: () => void;
  readonly url: string;
  constructor(url: string) { this.url = url; FakeWebSocket.instances.push(this); }
  close() { this.onclose?.(); }
  sendSnapshot(state: unknown) { this.onmessage?.({ data: JSON.stringify({ type: 'snapshot', state }) }); }
}

describe('SignalInfoModel', () => {
  it('applies insight snapshots and clears its stream subscription on stop', () => {
    const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const originalWebSocket = Object.getOwnPropertyDescriptor(globalThis, 'WebSocket');
    Object.defineProperty(globalThis, 'window', { configurable: true, value: { location: { protocol: 'http:', host: 'localhost' } } });
    Object.defineProperty(globalThis, 'WebSocket', { configurable: true, value: FakeWebSocket });
    try {
      const model = new SignalInfoModel();
      model.start();
      const socket = FakeWebSocket.instances.at(-1)!;
      assert.equal(socket.url, 'ws://localhost/signals/stream');
      socket.sendSnapshot({ signal_dashboard: { best: [{ symbol: 'BTC-USDT', direction: 'bullish' }], history: [], performance: [], walkForward: [] }, auto_trader_dashboard: { missedOpportunityBands: [{ gate: 'strength', threshold: 0.5, horizonSeconds: 300, sampleCount: 10, timeToProfitSampleCount: 2, stopTargetSampleCount: 2 }] } });
      assert.equal(model.state$.value.best[0].symbol, 'BTC-USDT');
      assert.equal(model.state$.value.missedOpportunityBands[0].gate, 'strength');
      model.stop();
    } finally {
      if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow); else delete (globalThis as any).window;
      if (originalWebSocket) Object.defineProperty(globalThis, 'WebSocket', originalWebSocket); else delete (globalThis as any).WebSocket;
      FakeWebSocket.instances = [];
    }
  });
});
