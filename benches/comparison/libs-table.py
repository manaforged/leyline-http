import sys,os,math,collections,statistics as st
try:
    from scipy.stats import t as student
    tq=lambda df:float(student.ppf(0.975,df))
except ImportError:
    T975=[12.706,4.303,3.182,2.776,2.571,2.447,2.365,2.306,2.262,2.228,2.201,2.179,2.160,2.145,2.131,2.120,2.110,2.101,2.093,2.086,2.080,2.074,2.069,2.064,2.060,2.056,2.052,2.048,2.045,2.042]
    tq=lambda df:T975[df-1] if df<=30 else 1.96
IDENTITY={"proto":None,"profile":None,"timeout":None,"tls":None,"certificate_verification":None,"ca_sha256":None,"leyline_source_hash":None,"leyline_python_artifact_sha256":"leyline-python","leyline_node_artifact_sha256":"leyline-node"}

def receipts(path):
    p=os.path.join(os.path.dirname(path),"receipts.txt")
    if not os.path.exists(p): return None
    out={}
    for l in open(p):
        k,_,v=l.rstrip("\n").partition(" ")
        if k in IDENTITY: out[k]=v
    return out

args=sys.argv[1:];vs=None
if "--vs" in args:
    i=args.index("--vs")
    if i+1>=len(args): sys.exit("--vs needs a library name")
    vs=args[i+1]; del args[i:i+2]
runs=[];dirs=[];paced_runs=[]
for path in args:
    run=collections.defaultdict(dict); paced=collections.defaultdict(dict)
    for l in open(path):
        p=l.split()
        if len(p)==15:
            r,cell,lib,rate,secs,done,rps,c50,c99,c999,_,_,_,kib,task=p
            sent=int(rate)*(int(secs)+1)
            paced[(cell,lib)][r]=(float(c50),float(c99),float(c999),float(task)*1000/sent,float(rps)/int(rate),int(kib)/1024,int(rate))
            continue
        if len(p)!=11: continue
        r,cell,lib,m,ia,ca,wa,m2,ib,cb,wb=p; dn=int(m2)-int(m)
        run[(cell,lib)][r]=(dn*1e9/(int(wb)-int(wa)),(float(ib)-float(ia))/dn,(float(cb)-float(ca))*1000/dn)
    if paced: paced_runs.append(paced)
    if run or paced: runs.append(run); dirs.append(path)
    else: print(f"{path}: no rows; left out",file=sys.stderr)
if not runs: sys.exit("no rows")
if len(runs)>1:
    ids=[receipts(p) for p in dirs]
    missing=[p for p,i in zip(dirs,ids) if i is None]
    if missing: sys.exit("refusing to pool: no receipts.txt next to "+", ".join(missing))
    bad=False
    clients={lib for run in runs for _,lib in run}
    for k,client in IDENTITY.items():
        if client is not None and client not in clients: continue
        vals={p:i.get(k,"absent") for p,i in zip(dirs,ids)}
        if len(set(vals.values()))>1 or any(v=="absent" or v.startswith("unknown") for v in vals.values()):
            bad=True; print(f"refusing to pool: {k} differs or is unknown",file=sys.stderr)
            for p,v in vals.items(): print(f"  {p}: {v}",file=sys.stderr)
    if bad: sys.exit(1)

conns=collections.defaultdict(list)
for path in dirs:
    cpath=os.path.join(os.path.dirname(path),"conns.txt")
    if os.path.exists(cpath):
        for l in open(cpath):
            c,lib,k=l.split()
            if k not in conns[(c,lib)]: conns[(c,lib)].append(k)
peak=collections.defaultdict(dict)
for n,path in enumerate(dirs):
    rpath=os.path.join(os.path.dirname(path),"rss.txt")
    if os.path.exists(rpath):
        for l in open(rpath):
            r,c,lib,kib=l.split()
            key=(n,r)
            peak[(c,lib)][key]=max(peak[(c,lib)].get(key,0),int(kib))
rss=lambda k:f"{st.median(peak[k].values())/1024:8.1f}" if peak.get(k) else f"{'-':>8}"
keys=list(dict.fromkeys(k for run in runs for k in run))
cells=list(dict.fromkeys(c for c,_ in keys))
pooled={k:[v for run in runs for v in run.get(k,{}).values()] for k in keys}
med=lambda k,i:st.median(x[i] for x in pooled[k])
def spread(k):
    s=[100*(max(v)-min(v))/st.median(v) for v in ([x[0] for x in run[k].values()] for run in runs if k in run)]
    return st.median(s)
if keys: print(f"{len(runs)} runs; req/s = (2m - m) / (wall_2m - wall_m); medians pooled over runs and rounds; spread = median within-run (max - min) / median")
for c in cells:
    libs=[l for (cc,l) in keys if cc==c]
    base="h2" if (c,"h2") in pooled else "floor" if (c,"floor") in pooled else None
    floor=med((c,base),1) if base else None
    print(f"\n{c:6} {'lib':15} {'req/s':>10} {'spread':>7} {'instr/req':>10} {'over '+(base or '-'):>10} {'cpu_us/req':>10} {'rss_MiB':>8} {'rounds':>6} {'conns':>5}")
    for l in sorted(libs,key=lambda l:-med((c,l),0)):
        v=pooled[(c,l)]; ins=med((c,l),1)
        over=f"{ins-floor:+10.0f}" if base else f"{'-':>10}"
        print(f"{'':6} {l:15} {med((c,l),0):10.0f} {spread((c,l)):6.1f}% {ins:10.0f} {over} {med((c,l),2):10.2f} {rss((c,l))} {len(v):6} {'/'.join(conns.get((c,l),['-'])):>5}")

def ratio_ci(ours,theirs,i,runs=runs):
    logs=[[math.log(run[ours][r][i]/run[theirs][r][i]) for r in run[ours] if r in run[theirs]] for run in runs if ours in run and theirs in run]
    logs=[p for p in logs if p]
    if not logs: return None
    if len(logs)>=3: xs=[st.mean(p) for p in logs]; label=f"{len(logs)} runs"
    else: xs=[x for p in logs for x in p]; label=f"single-run, {len(xs)} rounds"
    m=st.mean(xs)
    if len(xs)<2: return math.exp(m),None,None,label
    h=tq(len(xs)-1)*st.stdev(xs)/math.sqrt(len(xs))
    return math.exp(m),math.exp(m-h),math.exp(m+h),label

def show(r,up,down,same):
    g,lo,hi,_=r
    if lo is None: return f"{g:.3f} [no interval]"
    return f"{g:.3f} [{lo:.3f}, {hi:.3f}] "+(up if lo>1 else down if hi<1 else same)

if keys: print("\ngeometric-mean paired ratio (same run, same round); 95% t-interval on log ratio: across per-run means with >=3 runs, across rounds of one run otherwise (single-run, not cross-run evidence)")
for c in cells:
    libs=[l for (cc,l) in keys if cc==c]
    rivals=[l for l in libs if not l.startswith("leyline") and l!="floor"]
    against=([max(rivals,key=lambda l:med((c,l),0))]+(["floor"] if (c,"floor") in pooled else []) if rivals else [])
    if vs in libs and vs not in against: against.append(vs)
    for l in [l for l in libs if l.startswith("leyline")]:
        for name in [n for n in against if n!=l]:
            rps=ratio_ci((c,l),(c,name),0); cpu=ratio_ci((c,l),(c,name),2)
            if rps is None: print(f"{c:6} {l:17} vs {name:10} no paired rounds"); continue
            print(f"{c:6} {l:17} vs {name:10} req/s {show(rps,'ahead','behind','tie')}; CPU/req {show(cpu,'more CPU','less CPU','same CPU')}; {rps[3]}")

if paced_runs:
    pk=list(dict.fromkeys(k for run in paced_runs for k in run))
    pp={k:[v for run in paced_runs for v in run.get(k,{}).values()] for k in pk}
    pmed=lambda k,i:st.median(x[i] for x in pp[k])
    print("\nopen-loop cells: latency from the scheduled send time when the worker was behind it (coordinated omission), else from the send; first second left out; CPU per request at the same offered rate; medians over runs and rounds; lower is better")
    for c in dict.fromkeys(c for c,_ in pk):
        libs=[l for (cc,l) in pk if cc==c]
        print(f"\n{c:8} {'lib':15} {'rate':>8} {'achieved':>8} {'p50_us':>8} {'p99_us':>8} {'p999_us':>8} {'cpu_us/req':>10} {'rss_MiB':>8} {'rounds':>6}")
        for l in sorted(libs,key=lambda l:pmed((c,l),1)):
            k=(c,l); short=" short" if pmed(k,4)<0.95 else ""
            print(f"{'':8} {l:15} {pmed(k,6):8.0f} {pmed(k,4):8.3f} {pmed(k,0):8.0f} {pmed(k,1):8.0f} {pmed(k,2):8.0f} {pmed(k,3):10.2f} {pmed(k,5):8.1f} {len(pp[k]):6}{short}")
    print("\nopen-loop paired ratio, leyline over rival (same run, same round); 95% t-interval on log ratio; below 1 means leyline is lower")
    for c in dict.fromkeys(c for c,_ in pk):
        libs=[l for (cc,l) in pk if cc==c]
        for l in [l for l in libs if l.startswith("leyline")]:
            for name in [n for n in libs if not n.startswith("leyline")]:
                p99=ratio_ci((c,l),(c,name),1,paced_runs); cpu=ratio_ci((c,l),(c,name),3,paced_runs)
                if p99 is None: continue
                print(f"{c:8} {l:17} vs {name:10} p99 {show(p99,'higher','lower','tie')}; CPU/req {show(cpu,'more CPU','less CPU','same CPU')}; {p99[3]}")
