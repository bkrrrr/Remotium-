// Tiny HTTP server for trying terminals, port forwarding and the debugger.
const http = require('http');

function greeting(path) {
	const name = path.replace(/^\/+|\/+$/g, '') || 'world';
	return `hello, ${name}\n`; // set a breakpoint here
}

http.createServer((req, res) => {
	res.writeHead(200, { 'content-type': 'text/plain' });
	res.end(greeting(req.url));
}).listen(3000, '127.0.0.1', () => console.log('listening on http://127.0.0.1:3000'));
