import * as vscode from 'vscode';
import WebSocket from 'ws';

/** Above this many queued bytes, `drain()` waits. ManagedMessagePassing has no other backpressure signal. */
const HIGH_WATER = 8 * 1024 * 1024;
const LOW_WATER = 1 * 1024 * 1024;
const PING_INTERVAL_MS = 15_000;
/** No pong for this long: the connection is dead even if TCP hasn't noticed (NAT, proxies). */
const PONG_TIMEOUT_MS = 40_000;

export interface WsConnectOptions {
	headers?: Record<string, string>;
	/** For certificate pinning (see hostApi.ts). */
	agent?: import('http').Agent;
	log: vscode.LogOutputChannel;
	label: string;
	/** Called once the stream is closed after it was open. */
	onClosed?: (error?: Error) => void;
}

/** Error raised when the WebSocket could not be opened, with the HTTP status or WS close code. */
export class WsConnectError extends Error {
	constructor(message: string, readonly status?: number, readonly closeCode?: number) {
		super(message);
	}
}

/**
 * An event that buffers fired values until the first listener subscribes, so no bytes are
 * lost between `open` and the editor attaching its handler.
 */
class BufferedEmitter<T> implements vscode.Disposable {
	private readonly emitter = new vscode.EventEmitter<T>();
	private buffer: T[] | undefined = [];

	readonly event: vscode.Event<T> = (listener, thisArgs, disposables) => {
		const d = this.emitter.event(listener, thisArgs, disposables);
		const pending = this.buffer;
		this.buffer = undefined;
		pending?.forEach(v => this.emitter.fire(v));
		return d;
	};

	fire(v: T): void {
		if (this.buffer) {
			this.buffer.push(v);
		} else {
			this.emitter.fire(v);
		}
	}

	dispose(): void {
		this.emitter.dispose();
	}
}

/** Opens a binary WebSocket and exposes it as a `ManagedMessagePassing` byte stream. */
export function connectWs(url: string, opts: WsConnectOptions): Promise<vscode.ManagedMessagePassing> {
	const { log, label } = opts;
	return new Promise((resolve, reject) => {
		const ws = new WebSocket(url, { headers: opts.headers, agent: opts.agent, perMessageDeflate: false, handshakeTimeout: 30_000 });
		ws.binaryType = 'nodebuffer';
		const onMessage = new BufferedEmitter<Uint8Array>();
		const onClose = new BufferedEmitter<Error | undefined>();
		const onEnd = new BufferedEmitter<void>();
		let opened = false;
		let pinger: NodeJS.Timeout | undefined;
		let warnedBuffer = false;

		ws.on('unexpected-response', (_req, res) => {
			reject(new WsConnectError(`${label}: HTTP ${res.statusCode} ${res.statusMessage ?? ''}`.trim(), res.statusCode));
			ws.terminate();
		});

		ws.on('open', () => {
			opened = true;
			log.debug(`${label}: websocket open`);
			let lastPong = Date.now();
			ws.on('pong', () => (lastPong = Date.now()));
			ws.on('message', () => (lastPong = Date.now()));
			pinger = setInterval(() => {
				if (ws.readyState !== WebSocket.OPEN) {
					return;
				}
				if (Date.now() - lastPong > PONG_TIMEOUT_MS) {
					log.warn(`${label}: no pong for ${PONG_TIMEOUT_MS / 1000} s, dropping the connection`);
					ws.terminate();
					return;
				}
				ws.ping();
			}, PING_INTERVAL_MS);
			resolve({
				onDidReceiveMessage: onMessage.event,
				onDidClose: onClose.event,
				onDidEnd: onEnd.event,
				send: (data: Uint8Array) => {
					if (ws.readyState !== WebSocket.OPEN) {
						return;
					}
					ws.send(data, { binary: true });
					if (ws.bufferedAmount > HIGH_WATER && !warnedBuffer) {
						warnedBuffer = true;
						log.warn(`${label}: send buffer above ${HIGH_WATER} bytes (${ws.bufferedAmount}); the network is slower than the editor`);
					}
				},
				end: () => {
					log.debug(`${label}: end requested`);
					ws.close(1000);
				},
				drain: () => waitForDrain(ws),
			});
		});

		ws.on('message', (data: WebSocket.RawData) => {
			onMessage.fire(toUint8Array(data));
		});

		ws.on('close', (code, reason) => {
			clearInterval(pinger);
			const why = reason.toString();
			if (!opened) {
				reject(new WsConnectError(`${label}: closed before open (${code}${why ? ` ${why}` : ''})`, undefined, code));
				return;
			}
			log.debug(`${label}: websocket closed ${code} ${why}`);
			const err = code === 1000 || code === 1005 ? undefined : new WsConnectError(`websocket closed: ${code} ${why}`, undefined, code);
			onEnd.fire();
			onClose.fire(err);
			opts.onClosed?.(err);
			onMessage.dispose();
		});

		ws.on('error', err => {
			if (!opened) {
				reject(new WsConnectError(`${label}: ${err.message}`));
			} else {
				log.warn(`${label}: websocket error: ${err.message}`);
			}
		});
	});
}

function waitForDrain(ws: WebSocket): Promise<void> {
	if (ws.bufferedAmount <= HIGH_WATER) {
		return Promise.resolve();
	}
	return new Promise(resolve => {
		const t = setInterval(() => {
			if (ws.bufferedAmount <= LOW_WATER || ws.readyState !== WebSocket.OPEN) {
				clearInterval(t);
				resolve();
			}
		}, 10);
	});
}

function toUint8Array(data: WebSocket.RawData): Uint8Array {
	if (Array.isArray(data)) {
		return Buffer.concat(data);
	}
	if (data instanceof ArrayBuffer) {
		return new Uint8Array(data);
	}
	return data;
}
