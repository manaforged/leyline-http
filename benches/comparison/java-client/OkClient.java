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
import java.util.concurrent.TimeUnit;
import javax.net.ssl.SSLContext;
import javax.net.ssl.TrustManager;
import javax.net.ssl.TrustManagerFactory;
import javax.net.ssl.X509TrustManager;
import okhttp3.ConnectionPool;
import okhttp3.Dispatcher;
import okhttp3.OkHttpClient;
import okhttp3.Protocol;
import okhttp3.Request;
import okhttp3.Response;

public class OkClient {
    static final String NAME = "okhttp";
    static SSLContext ssl;
    static X509TrustManager trust;
    static byte[] expected;
    static Request request;
    static int maxRequests = 64;
    static boolean h1;

    static void initTls() throws Exception {
        String ca = System.getenv("CMP_CA");
        ssl = SSLContext.getInstance("TLS");
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
            trust = (X509TrustManager) tmf.getTrustManagers()[0];
        } else {
            trust = new X509TrustManager() {
                public void checkClientTrusted(X509Certificate[] c, String a) {}
                public void checkServerTrusted(X509Certificate[] c, String a) {}
                public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
            };
        }
        ssl.init(null, new TrustManager[] {trust}, new SecureRandom());
    }

    static OkHttpClient build() {
        Dispatcher dispatcher = new Dispatcher();
        dispatcher.setMaxRequests(maxRequests);
        dispatcher.setMaxRequestsPerHost(maxRequests);
        OkHttpClient.Builder b = new OkHttpClient.Builder()
            .protocols(h1 ? List.of(Protocol.HTTP_1_1) : List.of(Protocol.HTTP_2, Protocol.HTTP_1_1))
            .connectionPool(new ConnectionPool(h1 ? maxRequests : 1, 5, TimeUnit.MINUTES))
            .dispatcher(dispatcher)
            .retryOnConnectionFailure(false)
            .sslSocketFactory(ssl.getSocketFactory(), trust);
        if (System.getenv("CMP_CA") == null) b.hostnameVerifier((host, session) -> true);
        return b.build();
    }

    static void close(OkHttpClient client) {
        client.dispatcher().executorService().shutdown();
        client.connectionPool().evictAll();
    }

    static void get(OkHttpClient client) throws Exception {
        try (Response resp = client.newCall(request).execute()) {
            if (resp.code() != 200) throw new IllegalStateException("status " + resp.code());
            if (resp.protocol() != (h1 ? Protocol.HTTP_1_1 : Protocol.HTTP_2)) throw new IllegalStateException("protocol " + resp.protocol());
            if (!Arrays.equals(resp.body().bytes(), expected)) throw new IllegalStateException("response body mismatch");
        }
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
        String url = args.length > 0 ? args[0] : "https://127.0.0.1:8443/";
        request = new Request.Builder().url(url).get().build();

        if (args.length > 1 && args[1].equals("equiv")) {
            OkHttpClient client = build();
            try (Response resp = client.newCall(request).execute()) {
                byte[] bytes = resp.body().bytes();
                System.out.printf("EQUIV %s status=%d fnv=%016x len=%d%n", NAME, resp.code(), fnv1a(bytes), bytes.length);
            }
            close(client);
            System.exit(0);
        }

        int warmN = arg(args, 1, 2000);
        int coldN = arg(args, 2, 200);
        int concN = arg(args, 3, 20000);
        int concC = arg(args, 4, 64);
        maxRequests = Math.max(64, concC);
        String connEnv = System.getenv("CMP_CONNECTIONS");
        int connections = Math.max(1, connEnv == null ? 1 : Integer.parseInt(connEnv));
        List<OkHttpClient> clients = new ArrayList<>();
        for (int i = 0; i < connections; i++) clients.add(build());
        for (OkHttpClient c : clients) get(c);

        OkHttpClient client = clients.get(0);
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
                OkHttpClient c = clients.get(w % connections);
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
            OkHttpClient fresh = build();
            get(fresh);
            close(fresh);
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
