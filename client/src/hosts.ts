import * as vscode from 'vscode';

/** A paired remotium-host. The device token is kept in SecretStorage. */
export interface HostRecord {
	id: string;
	url: string;
	name: string;
	folder: string;
	fingerprint?: string;
	deviceId: string;
}

const HOSTS_KEY = 'remotium.hosts';
const tokenKey = (id: string) => `remotium.host.${id}`;

export class HostStore {
	constructor(private readonly ctx: vscode.ExtensionContext, private readonly log: vscode.LogOutputChannel) {}

	list(): HostRecord[] {
		return Object.values(this.ctx.globalState.get<Record<string, HostRecord>>(HOSTS_KEY, {}));
	}

	get(id: string): HostRecord | undefined {
		return this.ctx.globalState.get<Record<string, HostRecord>>(HOSTS_KEY, {})[id];
	}

	async add(h: HostRecord, token: string): Promise<void> {
		await this.ctx.secrets.store(tokenKey(h.id), token);
		await this.ctx.globalState.update(HOSTS_KEY, { ...this.ctx.globalState.get<Record<string, HostRecord>>(HOSTS_KEY, {}), [h.id]: h });
	}

	async remove(id: string): Promise<void> {
		const all = { ...this.ctx.globalState.get<Record<string, HostRecord>>(HOSTS_KEY, {}) };
		delete all[id];
		await this.ctx.globalState.update(HOSTS_KEY, all);
		await this.ctx.secrets.delete(tokenKey(id));
	}

	/**
	 * The host and its token. The window that handled the pairing link stores them and opens
	 * this window right away; values written there can take a moment to show up here.
	 */
	async load(id: string, timeoutMs = 10_000): Promise<{ host: HostRecord; token: string } | undefined> {
		const end = Date.now() + timeoutMs;
		let waited = false;
		for (;;) {
			const host = this.get(id);
			const token = await this.ctx.secrets.get(tokenKey(id));
			if (host && token) {
				if (waited) {
					this.log.info(`host ${id} became available after waiting`);
				}
				return { host, token };
			}
			if (Date.now() > end) {
				return undefined;
			}
			if (!waited) {
				this.log.info(`host ${id} not known in this window yet; waiting for the pairing from another window`);
				waited = true;
			}
			await new Promise(r => setTimeout(r, 300));
		}
	}
}
