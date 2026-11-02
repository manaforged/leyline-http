import java.io.FileInputStream;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
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

public class JdkClient {
    static final String NAME = "jdk-httpclient";
    static SSLContext ssl;
    static byte[] expected;
    static HttpRequest request;
    static HttpClient.Version version = HttpClient.Version.HTTP_2;

    static SSLContext sslContext() throws Exception {
        String ca = System.getenv("CMP_CA");
        SSLContext ctx = SSLContext.getInstance("TLS");
        if (ca != null) {
            X509Certificate cert;
            try (FileInputStream in = new FileInputStream(ca)) {
                cert = (X509Certificate) CertificateFactory.getInstance("X.509").generateCertificate(in);
            }
            KeyStore ks = KeyStore.getInstance(KeyStore.getDefaultType());
            ks.load(null, null);
            ks.setCertificateEntry("ca", cert);
            TrustManagerFactory tmf = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
            tmf.init(ks);
            ctx.init(null, tmf.getTrustManagers(), new SecureRandom());
        } else {
            System.setProperty("jdk.internal.httpclient.disableHostnameVerification", "true");
            TrustManager all = new X509TrustManager() {
                public void checkClientTrusted(X509Certificate[] c, String a) {}
                public void checkServerTrusted(X509Certificate[] c, String a) {}
                public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
            };
            ctx.init(null, new TrustManager[] {all}, new SecureRandom());
        }
        return ctx;
    }

    static HttpClient build() {
        return HttpClient.newBuilder().version(version).sslContext(ssl).build();
    }

    static void get(HttpClient client) throws Exception {
        HttpResponse<byte[]> resp = client.send(request, HttpResponse.BodyHandlers.ofByteArray());
        if (resp.statusCode() != 200) throw new IllegalStateException("status " + resp.statusCode());
        if (resp.version() != version) throw new IllegalStateException("version " + resp.version());
        if (!Arrays.equals(resp.body(), expected)) throw new IllegalStateException("response body mismatch");
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
        if ("h1".equals(proto)) version = HttpClient.Version.HTTP_1_1;
        else if (proto != null && !proto.equals("h2")) throw new IllegalArgumentException("CMP_PROTO " + proto);
        ssl = sslContext();
        String body = System.getenv("CMP_BODY");
        if (body != null) {
            expected = new byte[Integer.parseInt(body)];
            for (int i = 0; i < expected.length; i++) expected[i] = (byte) (i % 251);
        } else {
            expected = "ok-10byte!".getBytes();
        }
        String url = args.length > 0 ? args[0] : "https://127.0.0.1:8443/";
        request = HttpRequest.newBuilder(URI.create(url)).GET().build();

        if (args.length > 1 && args[1].equals("equiv")) {
            try (HttpClient client = build()) {
                HttpResponse<byte[]> resp = client.send(request, HttpResponse.BodyHandlers.ofByteArray());
                System.out.printf("EQUIV %s status=%d fnv=%016x len=%d%n", NAME, resp.statusCode(), fnv1a(resp.body()), resp.body().length);
            }
            return;
        }

        int warmN = arg(args, 1, 2000);
        int coldN = arg(args, 2, 200);
        int concN = arg(args, 3, 20000);
        int concC = arg(args, 4, 64);
        String connEnv = System.getenv("CMP_CONNECTIONS");
        int connections = Math.max(1, connEnv == null ? 1 : Integer.parseInt(connEnv));
        List<HttpClient> clients = new ArrayList<>();
        for (int i = 0; i < connections; i++) clients.add(build());
        for (HttpClient c : clients) get(c);

        HttpClient client = clients.get(0);
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
                HttpClient c = clients.get(w % connections);
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
            try (HttpClient fresh = build()) {
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
