import java.io.FileInputStream;
import java.security.KeyStore;
import java.security.SecureRandom;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import javax.net.ssl.SSLContext;
import javax.net.ssl.TrustManager;
import javax.net.ssl.TrustManagerFactory;
import javax.net.ssl.X509TrustManager;
import org.apache.hc.client5.http.async.methods.SimpleHttpRequest;
import org.apache.hc.client5.http.async.methods.SimpleHttpResponse;
import org.apache.hc.client5.http.async.methods.SimpleRequestBuilder;
import org.apache.hc.client5.http.classic.methods.HttpGet;
import org.apache.hc.client5.http.impl.async.CloseableHttpAsyncClient;
import org.apache.hc.client5.http.impl.async.HttpAsyncClients;
import org.apache.hc.client5.http.impl.classic.CloseableHttpClient;
import org.apache.hc.client5.http.impl.classic.HttpClients;
import org.apache.hc.client5.http.impl.io.PoolingHttpClientConnectionManagerBuilder;
import org.apache.hc.client5.http.ssl.ClientTlsStrategyBuilder;
import org.apache.hc.client5.http.ssl.NoopHostnameVerifier;
import org.apache.hc.core5.http.HttpVersion;
import org.apache.hc.core5.http.ProtocolVersion;
import org.apache.hc.core5.http.io.entity.EntityUtils;
import org.apache.hc.core5.io.CloseMode;

public class ApacheClient {
    static final String NAME = "apache-httpclient";
    static SSLContext ssl;
    static boolean verify;
    static boolean h1;
    static byte[] expected;
    static String url;
    static int poolSize = 64;

    record Reply(int status, byte[] body, ProtocolVersion version) {}

    interface Client extends AutoCloseable {
        Reply get() throws Exception;
    }

    static void initTls() throws Exception {
        String ca = System.getenv("CMP_CA");
        verify = ca != null;
        ssl = SSLContext.getInstance("TLS");
        TrustManager[] trust;
        if (verify) {
            X509Certificate cert;
            try (FileInputStream in = new FileInputStream(ca)) {
                cert = (X509Certificate) CertificateFactory.getInstance("X.509").generateCertificate(in);
            }
            KeyStore ks = KeyStore.getInstance(KeyStore.getDefaultType());
            ks.load(null, null);
            ks.setCertificateEntry("ca", cert);
            TrustManagerFactory tmf = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
            tmf.init(ks);
            trust = tmf.getTrustManagers();
        } else {
            trust = new TrustManager[] {new X509TrustManager() {
                public void checkClientTrusted(X509Certificate[] c, String a) {}
                public void checkServerTrusted(X509Certificate[] c, String a) {}
                public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
            }};
        }
        ssl.init(null, trust, new SecureRandom());
    }

    static ClientTlsStrategyBuilder tls() {
        ClientTlsStrategyBuilder b = ClientTlsStrategyBuilder.create().setSslContext(ssl);
        if (!verify) b.setHostnameVerifier(NoopHostnameVerifier.INSTANCE);
        return b;
    }

    static Client build() {
        if (h1) {
            CloseableHttpClient http = HttpClients.custom()
                .setConnectionManager(PoolingHttpClientConnectionManagerBuilder.create()
                    .setTlsSocketStrategy(tls().buildClassic())
                    .setMaxConnTotal(poolSize)
                    .setMaxConnPerRoute(poolSize)
                    .build())
                .disableAutomaticRetries()
                .build();
            return new Client() {
                public Reply get() throws Exception {
                    return http.execute(new HttpGet(url), r -> new Reply(r.getCode(), EntityUtils.toByteArray(r.getEntity()), r.getVersion()));
                }
                public void close() { http.close(CloseMode.GRACEFUL); }
            };
        }
        CloseableHttpAsyncClient http = HttpAsyncClients.customHttp2().setTlsStrategy(tls().build()).build();
        http.start();
        return new Client() {
            public Reply get() throws Exception {
                SimpleHttpRequest req = SimpleRequestBuilder.get(url).build();
                SimpleHttpResponse r = http.execute(req, null).get();
                return new Reply(r.getCode(), r.getBodyBytes(), r.getVersion());
            }
            public void close() { http.close(CloseMode.GRACEFUL); }
        };
    }

    static void get(Client client) throws Exception {
        Reply r = client.get();
        if (r.status() != 200) throw new IllegalStateException("status " + r.status());
        ProtocolVersion want = h1 ? HttpVersion.HTTP_1_1 : HttpVersion.HTTP_2;
        if (!want.equals(r.version())) throw new IllegalStateException("version " + r.version());
        if (!Arrays.equals(r.body(), expected)) throw new IllegalStateException("response body mismatch");
    }

    static long fnv1a(byte[] bytes) {
        long h = 0xcbf29ce484222325L;
        for (byte b : bytes) {
            h ^= (b & 0xff);
            h *= 0x100000001b3L;
        }
        return h;
    }

    static long pct(long[] v, double p) {
        return v[(int) Math.round((v.length - 1) * p)];
    }

    static int arg(String[] args, int i, int def) {
        return args.length > i ? Integer.parseInt(args[i]) : def;
    }

    public static void main(String[] args) throws Exception {
        String proto = System.getenv("CMP_PROTO");
        h1 = "h1".equals(proto);
        if (!h1 && proto != null && !proto.equals("h2")) throw new IllegalArgumentException("CMP_PROTO " + proto);
        initTls();
        String body = System.getenv("CMP_BODY");
        if (body != null) {
            expected = new byte[Integer.parseInt(body)];
            for (int i = 0; i < expected.length; i++) expected[i] = (byte) (i % 251);
        } else {
            expected = "ok-10byte!".getBytes();
        }
        url = args.length > 0 ? args[0] : "https://127.0.0.1:8443/";

        if (args.length > 1 && args[1].equals("equiv")) {
            try (Client client = build()) {
                Reply r = client.get();
                System.out.printf("EQUIV %s status=%d fnv=%016x len=%d%n", NAME, r.status(), fnv1a(r.body()), r.body().length);
            }
            System.exit(0);
        }

        int warmN = arg(args, 1, 2000);
        int coldN = arg(args, 2, 200);
        int concN = arg(args, 3, 20000);
        int concC = arg(args, 4, 64);
        poolSize = Math.max(64, concC);
        String connEnv = System.getenv("CMP_CONNECTIONS");
        int connections = Math.max(1, connEnv == null ? 1 : Integer.parseInt(connEnv));
        List<Client> clients = new ArrayList<>();
        for (int i = 0; i < connections; i++) clients.add(build());
        for (Client c : clients) get(c);

        Client client = clients.get(0);
        long[] warm = new long[warmN];
        long start = System.nanoTime();
        for (int i = 0; i < warmN; i++) {
            long t0 = System.nanoTime();
            get(client);
            warm[i] = (System.nanoTime() - t0) / 1000;
        }
        double warmRps = warmN / ((System.nanoTime() - start) / 1e9);
        Arrays.sort(warm);

        int per = concN / concC;
        long[] conc = new long[per * concC];
        start = System.nanoTime();
        try (ExecutorService exec = Executors.newFixedThreadPool(concC)) {
            List<Future<?>> futures = new ArrayList<>();
            for (int w = 0; w < concC; w++) {
                Client c = clients.get(w % connections);
                int base = w * per;
                futures.add(exec.submit(() -> {
                    for (int i = 0; i < per; i++) {
                        long t0 = System.nanoTime();
                        get(c);
                        conc[base + i] = (System.nanoTime() - t0) / 1000;
                    }
                    return null;
                }));
            }
            for (Future<?> f : futures) f.get();
        }
        double concRps = (per * concC) / ((System.nanoTime() - start) / 1e9);
        Arrays.sort(conc);

        start = System.nanoTime();
        for (int i = 0; i < coldN; i++) {
            try (Client fresh = build()) {
                get(fresh);
            }
        }
        double coldRps = coldN / ((System.nanoTime() - start) / 1e9);

        System.out.printf(
            "RESULT %s connections=%d warm_rps=%.3f conc_rps=%.3f cold_rps=%.3f warm_p50_us=%d warm_p90_us=%d warm_p99_us=%d conc_p50_us=%d conc_p90_us=%d conc_p99_us=%d conc_p999_us=%d%n",
            NAME, connections, warmRps, concRps, coldRps,
            pct(warm, 0.50), pct(warm, 0.90), pct(warm, 0.99),
            pct(conc, 0.50), pct(conc, 0.90), pct(conc, 0.99), pct(conc, 0.999));
        System.exit(0);
    }
}
