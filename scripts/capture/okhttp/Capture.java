import java.io.IOException;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;

/** GET tls.peet.ws/api/all with this classpath's OkHttp. */
public final class Capture {
    public static void main(String[] args) throws IOException {
        String url = args.length > 0 ? args[0] : "https://tls.peet.ws/api/all";
        OkHttpClient client = new OkHttpClient();
        Request request = new Request.Builder().url(url).build();
        try (Response response = client.newCall(request).execute()) {
            if (response.body() == null) {
                throw new IOException("empty body status=" + response.code());
            }
            System.out.print(response.body().string());
        }
    }

    private Capture() {}
}
