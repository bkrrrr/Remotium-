// HTTPS to a remotium-host, with certificate pinning for self-signed hosts.
import * as http from 'http';
import * as https from 'https';
import * as net from 'net';
import * as tls from 'tls';
import { normalizeFingerprint } from './pairing';

export class HostError extends Error {
	constructor(readonly status: number, readonly code: string, message: string) {
		super(message);
	}
	/** The device is unknown to the host (revoked, or the host's data was reset). */
	get isAuthFailure(): boolean {
		return this.status === 401;
	}
}

export interface HostTarget {
	url: string;
	fingerprint?: string;
}

/**
 * An agent whose connections are only handed out after the server's certificate matched the
 * pinned SHA-256. The check runs right after the TLS handshake, before any request bytes
 * (and the bearer token) are written.
 */
class PinnedAgent extends https.Agent {
	constructor(private readonly fingerprint: string) {
		super({ keepAlive: false });
	}

	override createConnection(options: any, callback?: (err: Error | null, stream: import('stream').Duplex) => void): any {
		const host: string = options.host ?? options.hostname;
		const s = tls.connect({
			host,
			port: options.port,
			// Node rejects IP addresses as SNI names.
			...(net.isIP(host?.replace(/^\[|\]$/g, '') ?? '') ? {} : { servername: options.servername ?? host }),
			rejectUnauthorized: false,
			ALPNProtocols: options.ALPNProtocols,
		});
		const fail = (e: Error) => callback?.(e, undefined as any);
		s.once('error', fail);
		s.once('secureConnect', () => {
			s.removeListener('error', fail);
			const got = normalizeFingerprint(s.getPeerCertificate().fingerprint256 ?? '');
			if (got !== this.fingerprint) {
				s.destroy();
				callback?.(new HostError(0, 'certificate', `the host's certificate changed (got ${got}); pair again if this is expected`), undefined as any);
				return;
			}
			callback?.(null, s);
		});
		return undefined;
	}
}

const agents = new Map<string, PinnedAgent>();

/** The agent to use for a host: pinned for self-signed hosts, default validation otherwise. */
export function agentFor(target: HostTarget): http.Agent | undefined {
	if (!target.url.startsWith('https:') || !target.fingerprint) {
		return undefined;
	}
	const fp = normalizeFingerprint(target.fingerprint);
	let a = agents.get(fp);
	if (!a) {
		a = new PinnedAgent(fp);
		agents.set(fp, a);
	}
	return a;
}

export function call<T>(target: HostTarget, method: string, path: string, body?: unknown, bearer?: string, timeoutMs = 20_000): Promise<T> {
	const u = new URL(path.replace(/^\//, ''), target.url.endsWith('/') ? target.url : `${target.url}/`);
	const payload = body === undefined ? undefined : Buffer.from(JSON.stringify(body));
	const headers: Record<string, string> = { accept: 'application/json' };
	if (payload) {
		headers['content-type'] = 'application/json';
		headers['content-length'] = String(payload.length);
	}
	if (bearer) {
		headers.authorization = `Bearer ${bearer}`;
	}
	const mod = u.protocol === 'https:' ? https : http;
	return new Promise((resolve, reject) => {
		const req = mod.request(u, { method, headers, agent: agentFor(target), timeout: timeoutMs }, res => {
			const chunks: Buffer[] = [];
			res.on('data', c => chunks.push(c));
			res.on('end', () => {
				const text = Buffer.concat(chunks).toString('utf8');
				let json: any = {};
				try {
					json = text ? JSON.parse(text) : {};
				} catch {
					json = { error: 'bad_response', message: text.slice(0, 200) };
				}
				const status = res.statusCode ?? 0;
				if (status >= 200 && status < 300) {
					resolve(json as T);
				} else {
					reject(new HostError(status, json.error ?? 'error', json.message ?? `HTTP ${status}`));
				}
			});
		});
		req.on('timeout', () => req.destroy(new HostError(0, 'network', `no answer from ${target.url} within ${timeoutMs / 1000} s`)));
		req.on('error', e => reject(e instanceof HostError ? e : new HostError(0, 'network', `cannot reach ${target.url}: ${e.message}`)));
		req.end(payload);
	});
}

export interface PairResult {
	deviceToken: string;
	deviceId: string;
	hostId: string;
	name: string;
	folder: string;
}

export const pair = (t: HostTarget, code: string, deviceName: string) =>
	call<PairResult>(t, 'POST', '/remotium/v1/pair', { code, deviceName });

export const createSession = (t: HostTarget, token: string, v: { commit: string; quality?: string; version: string; release?: string }) =>
	// May wait for the host to download a matching VSCodium server.
	call<{ connectionToken: string }>(t, 'POST', '/remotium/v1/session', v, token, 11 * 60_000);
