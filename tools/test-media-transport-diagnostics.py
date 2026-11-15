import importlib.util, io, json, os, pathlib, socket, ssl, subprocess, sys, unittest, urllib.error
from unittest.mock import Mock, patch

sys.dont_write_bytecode=True
source=pathlib.Path(os.environ.get('MEDIA_MCP_SOURCE', str(pathlib.Path(__file__).resolve().parents[1] / 'installer-pack/mcp/media-generation-mcp.py')))
spec=importlib.util.spec_from_file_location('media_under_test',source)
media=importlib.util.module_from_spec(spec)
spec.loader.exec_module(media)

class TransportDiagnostics(unittest.TestCase):
    def setUp(self):
        self.opener=Mock()
        for name,value in [('_api_key','secret-test-key'),('_api_profile','responses'),('_endpoint','https://example.invalid/v1/responses')]:
            p=patch.object(media,name,return_value=value); p.start(); self.addCleanup(p.stop)
        p=patch.object(media.urllib.request,'build_opener',return_value=self.opener); p.start(); self.addCleanup(p.stop)

    def invoke(self,method='POST'):
        with self.assertRaises(media.MediaError) as caught:
            media._http_json(method,'/v1/responses',{'prompt':'private-test-prompt'},'image-2')
        return caught.exception

    def test_connection_failure_retains_os_code_without_claiming_delivery(self):
        reason=OSError(10060,'private-test-prompt secret-test-key')
        reason.winerror=10060
        self.opener.open.side_effect=urllib.error.URLError(reason)
        err=self.invoke()
        diagnostic=err.details['transport_diagnostic']
        self.assertEqual(diagnostic['delivery_evidence'],'unconfirmed')
        self.assertEqual(diagnostic['winerror'],10060)
        self.assertEqual(err.submission_state,'submission_unknown')
        self.assertFalse(err.retryable)
        self.opener.open.assert_called_once()
        envelope=json.dumps(media._media_error_envelope(err),ensure_ascii=False)
        self.assertNotIn('secret-test-key',envelope)
        self.assertNotIn('private-test-prompt',envelope)

    def test_502_is_http_delivery_evidence_but_never_retried(self):
        self.opener.open.side_effect=urllib.error.HTTPError('https://example.invalid/v1/responses',502,'bad gateway',{'x-request-id':'test-request-id'},io.BytesIO(b'{"error":{"message":"Upstream service temporarily unavailable"}}'))
        err=self.invoke()
        self.assertEqual(err.provider_request_id,'test-request-id')
        self.assertEqual(err.details['transport_diagnostic']['delivery_evidence'],'http_response_received')
        self.assertEqual(err.details['transport_diagnostic']['http_status'],502)
        self.assertEqual(err.submission_state,'submission_unknown')
        self.opener.open.assert_called_once()

    def test_invalid_json_keeps_evidence_of_received_response(self):
        response=Mock()
        response.read.return_value=b'data: not-json'
        self.opener.open.return_value.__enter__=Mock(return_value=response)
        self.opener.open.return_value.__exit__=Mock(return_value=False)
        err=self.invoke()
        self.assertEqual(err.details['transport_diagnostic']['delivery_evidence'],'http_response_received')
        self.assertEqual(err.details['transport_diagnostic']['exception_type'],'JSONDecodeError')
        self.opener.open.assert_called_once()

    def test_read_only_failure_is_not_reported_as_generation_submission(self):
        self.opener.open.side_effect=urllib.error.URLError(socket.gaierror(-2,'not found'))
        err=self.invoke('GET')
        self.assertFalse(err.api_request_sent)
        self.assertTrue(err.retryable)
        self.assertEqual(err.details['transport_diagnostic']['cause_type'],'gaierror')
        self.opener.open.assert_called_once()

    def test_phase_trace_contains_only_bounded_transport_metadata(self):
        trace=media._TransportTrace()
        trace.mark('dns_resolution')
        trace.addresses=[{'family':'IPv4','ip':'203.0.113.7','port':443}]
        trace.mark('tcp_connect',family='IPv4',ip='203.0.113.7',port=443)
        trace.peer={'family':'IPv4','ip':'203.0.113.7','port':443}
        trace.request_bytes_written=246
        trace.mark('response_headers_wait')
        diagnostic=json.dumps(trace.snapshot(),ensure_ascii=False)
        self.assertIn('dns_resolution',diagnostic)
        self.assertIn('response_headers_wait',diagnostic)
        self.assertIn('203.0.113.7',diagnostic)
        self.assertNotIn('secret-test-key',diagnostic)
        self.assertNotIn('private-test-prompt',diagnostic)
        self.assertLessEqual(len(trace.snapshot()['events']),48)

    def test_tcp_failure_records_ip_family_and_no_http_bytes(self):
        trace=media._TransportTrace()
        conn=media._TracedHTTPSConnection('example.invalid',trace=trace,timeout=15)
        sock=Mock()
        failure=TimeoutError(10060,'timeout')
        failure.winerror=10060
        sock.connect.side_effect=failure
        addresses=[(media.socket.AF_INET,media.socket.SOCK_STREAM,6,'',('203.0.113.7',443))]
        with patch.object(media.socket,'getaddrinfo',return_value=addresses),patch.object(media.socket,'socket',return_value=sock):
            with self.assertRaises(TimeoutError):conn.connect()
        self.assertEqual(trace.phase,'tcp_connect_failed')
        self.assertEqual(trace.addresses,[{'family':'IPv4','ip':'203.0.113.7','port':443}])
        self.assertEqual(trace.request_bytes_written,0)
        sock.connect.assert_called_once_with(('203.0.113.7',443))
        sock.close.assert_called_once()

    def test_tls_timeout_distinguished_from_tcp_timeout(self):
        trace=media._TransportTrace()
        context=Mock()
        conn=media._TracedHTTPSConnection('example.invalid',trace=trace,context=context)
        raw=Mock()
        def connected(*args):
            trace.peer={'family':'IPv4','ip':'203.0.113.7','port':443}
            trace.mark('tcp_connected')
            return raw
        conn._create_connection=connected
        context.wrap_socket.side_effect=TimeoutError('handshake timeout')
        with self.assertRaises(TimeoutError):conn.connect()
        self.assertEqual(trace.phase,'tls_handshake')
        self.assertEqual(trace.request_bytes_written,0)
        self.assertEqual(trace.peer['ip'],'203.0.113.7')
        context.wrap_socket.assert_called_once_with(raw,server_hostname='example.invalid')

    def test_waiting_for_headers_records_completed_write(self):
        trace=media._TransportTrace()
        conn=media._TracedHTTPSConnection('example.invalid',trace=trace)
        sock=Mock()
        conn.sock=sock
        conn.send(b'test-header-bytes')
        self.assertEqual(trace.request_bytes_written,len(b'test-header-bytes'))
        with patch.object(media.http.client.HTTPSConnection,'getresponse',side_effect=TimeoutError('read timeout')):
            with self.assertRaises(TimeoutError):conn.getresponse()
        self.assertEqual(trace.phase,'response_headers_wait')
        self.assertFalse(trace.response_received)
        sock.sendall.assert_called_once_with(b'test-header-bytes')

class SimulatedSocket:
    """Exercise urllib/http.client without opening any real connection."""
    def __init__(self, body, status=200, fault=None):
        self.body, self.status, self.fault = body, status, fault
        self.writes, self.targets, self.timeouts, self.bindings = [], [], [], []
        self.closed = False

    def connect(self, address):
        self.targets.append(address)
        if self.fault == 'tcp':
            error = TimeoutError(10060, 'secret-test-key private-test-prompt')
            error.winerror = 10060
            raise error

    def settimeout(self, value): self.timeouts.append(value)
    def setsockopt(self, *args): pass
    def bind(self, address): self.bindings.append(address)
    def close(self): self.closed = True
    def version(self): return 'TLSv1.3'

    def sendall(self, data):
        if self.fault == 'write': raise TimeoutError('secret-test-key')
        self.writes.append(data)

    def makefile(self, mode):
        if self.fault == 'headers': raise TimeoutError('secret-test-key')
        wire = (f'HTTP/1.1 {self.status} Simulated\r\nContent-Length: {len(self.body)}\r\nConnection: close\r\n\r\n'.encode() + self.body)
        if self.fault in ('body', 'incomplete'):
            fault = self.fault
            class BrokenBody(io.BytesIO):
                def read(self, *args):
                    if fault == 'incomplete': raise media.http.client.IncompleteRead(b'private-test-prompt')
                    raise TimeoutError('secret-test-key')
            return BrokenBody(wire)
        return io.BytesIO(wire)


class CatalogTransport(unittest.TestCase):
    def setUp(self):
        self.body = b'{"data":[{"id":"gpt-image-2.5"}]}'
        self.status, self.fault = 200, None
        self.sockets = []
        self.addresses = [
            (socket.AF_INET6, socket.SOCK_STREAM, 6, '', ('2001:db8::7', 443, 0, 0)),
            (socket.AF_INET, socket.SOCK_STREAM, 6, '', ('203.0.113.7', 443)),
        ]
        self.context = Mock()
        self.context.wrap_socket.side_effect = self.wrap_socket
        replacements = [
            (media, '_api_key', {'return_value':'secret-test-key'}),
            (media, '_api_profile', {'return_value':'responses'}),
            (media, '_endpoint', {'return_value':'https://example.invalid/v1/models?token=private-query'}),
            (media, '_base', {'return_value':'https://example.invalid'}),
            (media, '_runtime_config', {'return_value':{}}),
            (media, '_config_path', {'return_value':pathlib.Path('simulated-catalog.json')}),
            (media, '_atomic_json', {}),
            (media.socket, 'getaddrinfo', {'side_effect':self.resolve}),
            (media.socket, 'socket', {'side_effect':self.new_socket}),
            (media.http.client, '_create_https_context', {'return_value':self.context}),
            (media.urllib.request, 'getproxies', {'return_value':{}}),
        ]
        self.mocks = {}
        for owner, name, kwargs in replacements:
            patcher = patch.object(owner, name, **kwargs)
            self.mocks[name] = patcher.start()
            self.addCleanup(patcher.stop)

    def resolve(self, *args):
        if self.fault == 'dns': raise socket.gaierror(-2, 'secret-test-key')
        return self.addresses

    def new_socket(self, *args):
        sock = SimulatedSocket(self.body, self.status, self.fault)
        self.sockets.append(sock)
        return sock

    def wrap_socket(self, sock, **kwargs):
        if self.fault == 'tls': raise ssl.SSLError('secret-test-key')
        return sock

    def diagnostic(self):
        with self.assertRaises(media.MediaError) as caught:
            media.media_refresh_models({})
        return caught.exception.details['transport_diagnostic']

    def assert_safe(self, diagnostic):
        serialized = json.dumps(diagnostic)
        for secret in ('secret-test-key', 'private-test-prompt', 'private-query', 'Authorization', 'Bearer', 'data'):
            self.assertNotIn(secret, serialized)
        self.assertEqual(diagnostic['endpoint'], '/v1/models')
        self.assertEqual(diagnostic['revision'], media.TRANSPORT_DIAGNOSTICS_REVISION)
        for event in diagnostic['phase_trace']['events']:
            self.assertGreaterEqual(event['duration_ms'], 0)

    def test_catalog_success_returns_safe_trace_and_does_not_persist_it(self):
        result = media.media_refresh_models({})
        diagnostic = result['transport_diagnostic']
        self.assert_safe(diagnostic)
        self.assertTrue(result['success'])
        self.assertFalse(result['generation_request_sent'])
        self.assertEqual(result['catalog_count'], 1)
        self.assertEqual(diagnostic['http_status'], 200)
        self.assertTrue(diagnostic['request_attempted'])
        self.assertEqual(diagnostic['delivery_evidence'], 'http_response_received')
        trace = diagnostic['phase_trace']
        self.assertIsNone(trace['failure_phase'])
        self.assertEqual(trace['last_phase'], 'completed')
        self.assertEqual([a['family'] for a in trace['dns_addresses']], ['IPv6', 'IPv4'])
        self.assertEqual(trace['peer']['ip'], '2001:db8::7')
        self.assertEqual(trace['process']['pid'], os.getpid())
        self.assertEqual(trace['process']['executable'], sys.executable)
        phases = [event['phase'] for event in trace['events']]
        for phase in ('dns_resolution','tcp_connect','tls_handshake','http_write','response_headers_wait','response_body_read','completed'):
            self.assertIn(phase, phases)
        self.assertEqual(len(self.sockets), 1)
        self.assertEqual(len(self.sockets[0].writes), 1)
        self.assertEqual(self.sockets[0].timeouts, [media.HTTP_TIMEOUT_SECONDS])
        self.assertTrue(self.sockets[0].writes[0].startswith(b'GET /v1/models?token=private-query HTTP/1.1'))
        self.mocks['_atomic_json'].assert_called_once()
        self.assertNotIn('transport_diagnostic', self.mocks['_atomic_json'].call_args.args[1])

    def test_failure_stages_are_observed_without_http_retry(self):
        cases = [('dns','dns_resolution',False), ('tcp','tcp_connect_failed',False),
                 ('tls','tls_handshake',False), ('write','http_write',False),
                 ('headers','response_headers_wait',False), ('body','response_body_read',True),
                 ('incomplete','response_body_read',True)]
        for fault, phase, received in cases:
            with self.subTest(fault=fault):
                self.fault = fault
                self.sockets.clear()
                diagnostic = self.diagnostic()
                self.assert_safe(diagnostic)
                trace = diagnostic['phase_trace']
                self.assertEqual(trace['failure_phase'], phase)
                self.assertEqual(trace['http_response_received'], received)
                self.assertEqual(diagnostic['delivery_evidence'], 'http_response_received' if received else 'unconfirmed')
                self.assertLessEqual(sum(len(s.writes) for s in self.sockets), 1)
                if fault == 'tcp':
                    self.assertEqual(diagnostic['winerror'], 10060)
                    self.assertEqual([s.targets[0] for s in self.sockets], [a[4] for a in self.addresses])
                if fault in ('dns','tcp','tls'):
                    self.assertEqual(trace['request_bytes_written'], 0)
                self.mocks['_atomic_json'].assert_not_called()

    def test_http_error_and_invalid_responses_keep_received_evidence(self):
        cases = [(502,b'{"error":{"message":"temporary"}}'), (200,b'invalid-json'),
                 (200,b'\xff'), (200,b'[]'), (200,b'{}'), (200,b'{"data":{}}')]
        for status, body in cases:
            with self.subTest(status=status, body=body):
                self.status, self.body = status, body
                diagnostic = self.diagnostic()
                self.assert_safe(diagnostic)
                self.assertEqual(diagnostic['http_status'], status)
                self.assertEqual(diagnostic['delivery_evidence'], 'http_response_received')
                self.mocks['_atomic_json'].assert_not_called()

    def test_large_body_error_keeps_trace(self):
        with patch.object(media, 'MAX_JSON_BYTES', 4):
            self.assertEqual(self.diagnostic()['phase_trace']['failure_phase'], 'response_body_read_completed')

    def test_local_rejection_never_attempts_connection(self):
        def reject(): raise media.MediaError('local validation rejected')
        with self.assertRaises(media.MediaError) as caught:
            media._http_json('POST','/v1/responses',{'prompt':'private-test-prompt'},'image-2',before_request=reject)
        self.assertFalse(caught.exception.api_request_sent)
        self.assertFalse(caught.exception.details['transport_diagnostic']['request_attempted'])
        self.assertEqual(self.sockets, [])
        self.mocks['getaddrinfo'].assert_not_called()

    def test_query_secret_is_not_in_diagnostic(self):
        diagnostic = {}
        media._http_json('GET','/v1/models?token=private-query',None,'catalog',diagnostic_out=diagnostic)
        self.assert_safe(diagnostic)

    def test_all_candidates_are_attempted_even_when_metadata_is_bounded(self):
        self.fault = 'tcp'
        self.addresses = [self.addresses[1]] * 60
        diagnostic = self.diagnostic()
        trace = diagnostic['phase_trace']
        self.assertEqual(len(self.sockets), 60)
        self.assertEqual(trace['dns_candidate_count'], 60)
        self.assertEqual(len(trace['dns_addresses']), 16)
        self.assertEqual(len(trace['events']), 48)
        self.assertTrue(trace['events_truncated'])
        self.assertTrue(all(sock.closed for sock in self.sockets))
        self.assertEqual(sum(len(s.writes) for s in self.sockets), 0)

    def test_source_bind_and_proxy_tunnel_target_are_preserved(self):
        trace = media._TransportTrace()
        connection = media._TracedHTTPSConnection('proxy.invalid',trace=trace,context=self.context,timeout=17,source_address=('127.0.0.1',0))
        connection.set_tunnel('gateway.invalid',443)
        with patch.object(media.http.client.HTTPConnection,'_tunnel') as tunnel:
            connection.connect()
        tunnel.assert_called_once()
        self.assertEqual(self.mocks['getaddrinfo'].call_args.args[:2], ('proxy.invalid',443))
        self.assertEqual(self.sockets[0].timeouts, [17])
        self.assertEqual(self.sockets[0].bindings, [('127.0.0.1',0)])
        self.context.wrap_socket.assert_called_once_with(self.sockets[0],server_hostname='gateway.invalid')
        self.assertIn('proxy_tunnel_established', [e['phase'] for e in trace.events])
        connection.close()

    def test_snapshot_does_not_mutate_events(self):
        trace = media._TransportTrace()
        trace.mark('dns_resolution')
        snapshot = trace.snapshot()
        self.assertIn('duration_ms',snapshot['events'][0])
        self.assertNotIn('duration_ms',trace.events[0])


class ProcessIdentity(unittest.TestCase):
    def environment(self):
        environment = {k:v for k,v in os.environ.items() if not k.startswith('MEDIA_') and k != 'OPENAI_API_KEY'}
        environment['MEDIA_GENERATION_CONFIG_PATH'] = str(source.parent / '__offline_probe_no_config__.json')
        return environment

    def test_healthcheck_reports_revision_without_network(self):
        result = subprocess.run([sys.executable,'-B',str(source),'--healthcheck'],capture_output=True,env=self.environment(),timeout=15,check=True)
        health = json.loads(result.stdout)
        self.assertTrue(health['ready'])
        self.assertEqual(health['serverVersion'],'1.3.11')
        self.assertEqual(health['transportDiagnosticsRevision'],media.TRANSPORT_DIAGNOSTICS_REVISION)
        self.assertEqual(health['generationCapability'],'not_tested')

    def test_fresh_stdio_processes_report_loaded_revision_and_own_pid(self):
        requests = [
            {'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':media.PROTOCOL_VERSION}},
            {'jsonrpc':'2.0','id':2,'method':'tools/list','params':{}},
            {'jsonrpc':'2.0','id':3,'method':'tools/call','params':{'name':'media_capabilities','arguments':{}}},
        ]
        wire = ('\n'.join(json.dumps(r) for r in requests)+'\n').encode()
        processes = []
        for _ in range(2):
            process = subprocess.Popen([sys.executable,'-B',str(source)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=self.environment())
            out, err = process.communicate(wire,timeout=15)
            self.assertEqual(process.returncode,0)
            self.assertEqual(err,b'')
            replies = [json.loads(line) for line in out.splitlines()]
            self.assertEqual(len(replies[1]['result']['tools']),6)
            capability = json.loads(replies[2]['result']['content'][0]['text'])
            diagnostic = capability['transport_diagnostics']
            self.assertEqual(diagnostic['revision'],media.TRANSPORT_DIAGNOSTICS_REVISION)
            self.assertEqual(diagnostic['base_revision'],'phase-trace-v2')
            self.assertTrue(diagnostic['catalog_get_success_trace'])
            self.assertEqual(diagnostic['loaded_pid'],process.pid)
            self.assertEqual(diagnostic['python_executable'],sys.executable)
            processes.append(process.pid)
        self.assertNotEqual(*processes)


if __name__=='__main__': unittest.main(verbosity=2)
