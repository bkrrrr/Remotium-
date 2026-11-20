import * as vscode from 'vscode';

export type ConnState = 'resolving' | 'connected' | 'reconnecting' | 'error';

/** Status bar item for the remote window's Remotium connection. */
export class StatusItem implements vscode.Disposable {
	private readonly item = vscode.window.createStatusBarItem('remotium.status', vscode.StatusBarAlignment.Left, 10000);
	private open = 0;

	constructor(private readonly env: string) {
		this.item.name = 'Remotium';
		this.item.command = 'remotium.menu';
		this.set('resolving');
		this.item.show();
	}

	set(state: ConnState, detail?: string): void {
		const icon = { resolving: '$(sync~spin)', connected: '$(remote)', reconnecting: '$(sync~spin)', error: '$(error)' }[state];
		this.item.text = `${icon} Remotium: ${this.env}`;
		this.item.tooltip = `${state}${detail ? `: ${detail}` : ''} · ${this.open} open connection(s)`;
		this.item.backgroundColor = state === 'error' ? new vscode.ThemeColor('statusBarItem.errorBackground') : undefined;
	}

	opened(): void {
		this.open++;
		this.set('connected');
	}

	closed(error?: Error): void {
		this.open = Math.max(0, this.open - 1);
		if (error) {
			this.set('reconnecting', error.message);
		} else if (this.open > 0) {
			this.set('connected');
		}
	}

	dispose(): void {
		this.item.dispose();
	}
}
