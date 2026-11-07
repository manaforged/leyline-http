using System.Diagnostics;
using System.Net;
using System.Net.Security;
using System.Security.Cryptography.X509Certificates;

const string Name = "dotnet-httpclient";

var proto = Environment.GetEnvironmentVariable("CMP_PROTO") ?? "h2";
if (proto != "h1" && proto != "h2")
{
    Console.Error.WriteLine($"CMP_PROTO {proto}");
    return 1;
}
var version = proto == "h1" ? HttpVersion.Version11 : HttpVersion.Version20;

var caPath = Environment.GetEnvironmentVariable("CMP_CA");
X509Certificate2? ca = caPath is null ? null : X509CertificateLoader.LoadCertificateFromFile(caPath);

var bodyEnv = Environment.GetEnvironmentVariable("CMP_BODY");
byte[] expected = bodyEnv is null
    ? "ok-10byte!"u8.ToArray()
    : Enumerable.Range(0, int.Parse(bodyEnv)).Select(i => (byte)(i % 251)).ToArray();

var url = new Uri(args.Length > 0 ? args[0] : "https://127.0.0.1:8443/");
int Arg(int i, int def) => args.Length > i ? int.Parse(args[i]) : def;
var pool = Math.Max(64, args.Length > 4 && args[1] != "equiv" ? int.Parse(args[4]) : 64);

bool Validate(object sender, X509Certificate? cert, X509Chain? _, SslPolicyErrors errors)
{
    if (ca is null) return true;
    if (cert is not X509Certificate2 leaf || errors.HasFlag(SslPolicyErrors.RemoteCertificateNameMismatch)) return false;
    using var chain = new X509Chain();
    chain.ChainPolicy.TrustMode = X509ChainTrustMode.CustomRootTrust;
    chain.ChainPolicy.CustomTrustStore.Add(ca);
    chain.ChainPolicy.RevocationMode = X509RevocationMode.NoCheck;
    return chain.Build(leaf);
}

HttpClient Build() => new(new SocketsHttpHandler
{
    MaxConnectionsPerServer = proto == "h1" ? pool : 1,
    EnableMultipleHttp2Connections = false,
    SslOptions = new SslClientAuthenticationOptions { RemoteCertificateValidationCallback = Validate },
});

HttpRequestMessage Request() => new(HttpMethod.Get, url)
{
    Version = version,
    VersionPolicy = HttpVersionPolicy.RequestVersionExact,
};

async Task Get(HttpClient client)
{
    using var resp = await client.SendAsync(Request());
    var body = await resp.Content.ReadAsByteArrayAsync();
    if (resp.StatusCode != HttpStatusCode.OK) throw new InvalidOperationException($"status {(int)resp.StatusCode}");
    if (resp.Version != version) throw new InvalidOperationException($"version {resp.Version}");
    if (!body.AsSpan().SequenceEqual(expected)) throw new InvalidOperationException("response body mismatch");
}

static ulong Fnv1a(byte[] bytes)
{
    ulong h = 0xcbf29ce484222325UL;
    foreach (var b in bytes)
    {
        h ^= b;
        h *= 0x100000001b3UL;
    }
    return h;
}

static long Pct(long[] v, double p) => v[(int)Math.Round((v.Length - 1) * p, MidpointRounding.AwayFromZero)];

static long Micros(long t0) => (long)Stopwatch.GetElapsedTime(t0).TotalMicroseconds;

if (args.Length > 1 && args[1] == "equiv")
{
    using var client = Build();
    using var resp = await client.SendAsync(Request());
    var body = await resp.Content.ReadAsByteArrayAsync();
    Console.WriteLine($"EQUIV {Name} status={(int)resp.StatusCode} fnv={Fnv1a(body):x16} len={body.Length}");
    return 0;
}

int warmN = Arg(1, 2000), coldN = Arg(2, 200), concN = Arg(3, 20000), concC = Arg(4, 64);
var connEnv = Environment.GetEnvironmentVariable("CMP_CONNECTIONS");
int connections = Math.Max(1, connEnv is null ? 1 : int.Parse(connEnv));
var clients = Enumerable.Range(0, connections).Select(_ => Build()).ToList();
foreach (var c in clients) await Get(c);

var first = clients[0];
var warm = new long[warmN];
var start = Stopwatch.GetTimestamp();
for (int i = 0; i < warmN; i++)
{
    var t0 = Stopwatch.GetTimestamp();
    await Get(first);
    warm[i] = Micros(t0);
}
double warmRps = warmN / Stopwatch.GetElapsedTime(start).TotalSeconds;
Array.Sort(warm);

int per = concN / concC;
var conc = new long[per * concC];
start = Stopwatch.GetTimestamp();
await Task.WhenAll(Enumerable.Range(0, concC).Select(async w =>
{
    var c = clients[w % connections];
    int baseIdx = w * per;
    for (int i = 0; i < per; i++)
    {
        var t0 = Stopwatch.GetTimestamp();
        await Get(c);
        conc[baseIdx + i] = Micros(t0);
    }
}));
double concRps = per * concC / Stopwatch.GetElapsedTime(start).TotalSeconds;
Array.Sort(conc);

start = Stopwatch.GetTimestamp();
for (int i = 0; i < coldN; i++)
{
    using var fresh = Build();
    await Get(fresh);
}
double coldRps = coldN / Stopwatch.GetElapsedTime(start).TotalSeconds;

foreach (var c in clients) c.Dispose();

Console.WriteLine(FormattableString.Invariant(
    $"RESULT {Name} connections={connections} warm_rps={warmRps:F3} conc_rps={concRps:F3} cold_rps={coldRps:F3} warm_p50_us={Pct(warm, 0.50)} warm_p90_us={Pct(warm, 0.90)} warm_p99_us={Pct(warm, 0.99)} conc_p50_us={Pct(conc, 0.50)} conc_p90_us={Pct(conc, 0.90)} conc_p99_us={Pct(conc, 0.99)} conc_p999_us={Pct(conc, 0.999)}"));
return 0;
