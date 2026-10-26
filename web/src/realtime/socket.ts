// Browser end of the gateway's /ws channel.
//
// Each connection needs a fresh single-use ticket, so `start` fetches one every time.
// The client re-subscribes to the threads it wanted after a reconnect.

import { api } from "../api/client";

export type Frame = { type: string; [key: string]: unknown };
export type SocketStatus = "connecting" | "open" | "closed";

const RECONNECT_DELAY_MS = 2000;

export class GatewaySocket {
  private ws: WebSocket | null = null;
  private wanted = new Set<string>();
  private stopped = true;

  constructor(
    private readonly onFrame: (frame: Frame) => void,
    private readonly onStatus: (status: SocketStatus) => void,
  ) {}

  async start(): Promise<void> {
    this.stopped = false;
    this.onStatus("connecting");
    const { ticket } = await api<{ ticket: string }>("POST", "/ws-tickets");
    if (this.stopped) return;

    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const ws = new WebSocket(`${scheme}://${location.host}/ws?ticket=${encodeURIComponent(ticket)}`);
    this.ws = ws;
    ws.onopen = () => {
      this.onStatus("open");
      for (const threadId of this.wanted) this.send({ type: "subscribe", threadId });
    };
    ws.onmessage = (event) => this.onFrame(JSON.parse(event.data as string) as Frame);
    ws.onclose = () => {
      this.onStatus("closed");
      if (!this.stopped) {
        setTimeout(() => this.start().catch(() => this.onStatus("closed")), RECONNECT_DELAY_MS);
      }
    };
  }

  stop(): void {
    this.stopped = true;
    this.ws?.close();
    this.ws = null;
  }

  subscribe(threadId: string): void {
    this.wanted.add(threadId);
    this.send({ type: "subscribe", threadId });
  }

  unsubscribe(threadId: string): void {
    this.wanted.delete(threadId);
    this.send({ type: "unsubscribe", threadId });
  }

  answerApproval(requestId: string, result: unknown): void {
    this.send({ type: "approvalResponse", requestId, result });
  }

  private send(message: unknown): void {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(message));
  }
}
