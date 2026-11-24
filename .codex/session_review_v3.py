#!/usr/bin/env python3
"""Local, report-only Codex rollout audit. Standard library; no network."""
import argparse, collections as C, datetime as D, hashlib, json, math, os, re, tempfile
from pathlib import Path

RULES = {
 'read': (r'\b(cat|sed|head|tail|rtk read|rtk bulk-read)\b', 'Beads / repository navigation'),
 'search': (r'\b(rg|grep|ast-grep|find)\b', 'Beads / repository indexes'),
 'test': (r'\b(just test|cargo test|just check|cargo check|just typecheck|nx typecheck)\b', 'Regression evidence / build ownership'),
 'sql': (r'\b(sqlite3|SELECT\s+.+?\s+FROM|scripts/db/)\b', 'SQL catalogue'),
 'api': (r'\b(curl|rsocket|scripts/api/)\b', 'API tooling'),
 'python': (r'\bpython3?\b', 'Reusable deterministic scripts'),
 'existing_tool': (r'scripts/(local|agent|diagnostics|api|db)/', 'Tooling adoption'),
 'delegate': (r'\b(spawn_agent|delegate|send_input)\b', 'Agent profiles / delegation'),
}
CORRECTIONS = {
 'repeat': r'already (checked|read|done|said)|stop repeating|again\?|why.*again',
 'reuse': r'use (the |our )?existing|already (have|exists)|don.t (duplicate|reinvent)',
 'scope': r'don.t (restart|deploy|commit|push|change)|wrong (database|file|service)|not what I asked',
 'context': r'too (many tokens|much context|long)|stop searching|remember (where|this)',
}

def digest(s): return hashlib.sha256(s.encode()).hexdigest()[:16]
def content(p):
    x=p.get('content',p.get('message',''))
    if isinstance(x,list): return '\n'.join(str(i.get('text','')) for i in x if isinstance(i,dict))
    return x if isinstance(x,str) else ''
def quantile(xs,q): return sorted(xs)[max(0,math.ceil(len(xs)*q)-1)] if xs else None

def audit(paths, since, repo):
    totals=C.Counter(); events=C.Counter(); groups=C.Counter(); corrections=C.Counter(); failures=C.Counter(); large=C.Counter(); repeat=C.Counter(); samples=[]; seen=set(); sessions=[]; examples=C.defaultdict(list)
    for path in paths:
        calls={}; counts=C.Counter(); local=C.Counter(); last_total=None; prev_tool=None; session_id=None; duplicate=False; eligible=False; token_rows=[]; last_usage=None; session_start=None
        try:
            with path.open(encoding='utf-8',errors='replace') as f:
                for lineno,line in enumerate(f,1):
                    try: row=json.loads(line); p=row.get('payload',{}); assert isinstance(p,dict)
                    except (ValueError,AttributeError,AssertionError): totals['parse_errors']+=1; continue
                    typ=row.get('type'); pt=p.get('type'); ts=row.get('timestamp','')
                    if typ=='session_meta':
                        session_id=p.get('id'); cwd=p.get('cwd',''); session_start=ts
                        if session_id and session_id in seen: duplicate=True; break
                        if repo and (not isinstance(cwd,str) or not (cwd==repo or cwd.startswith(repo+'/'))): break
                        eligible=True
                    if not eligible:
                        if repo: continue
                        eligible=True
                    if since and isinstance(ts,str) and ts and ts[:10]<since: continue
                    events[str(typ)+'/'+str(pt)]+=1
                    if typ=='response_item' and pt in ('function_call','custom_tool_call'):
                        name=str(p.get('name','unknown')); raw=p.get('arguments',p.get('input','')); raw=raw if isinstance(raw,str) else json.dumps(raw,sort_keys=True)
                        try:
                            a=json.loads(raw); cmd=a.get('cmd',a.get('command',raw)) if isinstance(a,dict) else raw
                        except ValueError: cmd=raw
                        cmd=str(cmd); key=digest(name+'\0'+cmd); cats=[k for k,(pattern,_) in RULES.items() if re.search(pattern,cmd+' '+name,re.I|re.S)] or ['other']; counts[key]+=1; local['calls']+=1
                        for cat in cats:
                            groups[cat]+=1
                            if counts[key]>1: repeat[cat]+=1
                        calls[str(p.get('call_id',''))]=(cats,key); prev_tool=key
                    elif typ=='response_item' and pt in ('function_call_output','custom_tool_call_output'):
                        out=p.get('output',''); out=out if isinstance(out,str) else json.dumps(out); cats,key=calls.pop(str(p.get('call_id','')),(['unmatched'],'unknown')); local['outputs']+=1
                        if len(out)>20000:
                            local['large_outputs']+=1
                            for cat in cats: large[cat]+=1
                        if re.search(r'(?im)^(?:.*(?:exit code|exited with code)[: ]+[1-9]\d*|error\[E\d+\]|test result: FAILED|.*command not found)',out):
                            local['failure_candidates']+=1
                            for cat in cats: failures[cat]+=1
                    elif typ=='response_item' and pt=='message' and p.get('role')=='user':
                        msg=content(p); local['user_messages']+=1
                        for cat,pattern in CORRECTIONS.items():
                            if re.search(pattern,msg,re.I):
                                corrections[cat]+=1
                                if len(examples[cat])<5: examples[cat].append({'session':digest(str(session_id or path.name)),'line':lineno,'previous_tool_hash':prev_tool})
                    elif typ=='event_msg' and pt=='token_count':
                        info=p.get('info') or {}; usage=info.get('last_token_usage') or {}; cumulative=info.get('total_token_usage') or {}
                        if not isinstance(usage,dict) or not isinstance(cumulative,dict): continue
                        inp=usage.get('input_tokens'); cached=usage.get('cached_input_tokens',0); output=usage.get('output_tokens',0); reasoning=usage.get('reasoning_output_tokens',0)
                        if not all(isinstance(x,int) and x>=0 for x in (inp,cached,output,reasoning)): continue
                        signature=json.dumps(cumulative,sort_keys=True) if cumulative else None
                        if signature is None: local['usage_without_cumulative_skipped']+=1; continue
                        if signature==last_usage: continue
                        last_usage=signature; token_rows.append(inp); local['input_tokens']+=inp; local['cached_input_tokens']+=cached; local['output_tokens']+=output; local['reasoning_tokens']+=reasoning
        except OSError: totals['unreadable_files']+=1; continue
        if duplicate: totals['duplicate_session_files']+=1; continue
        if not eligible: continue
        if session_id: seen.add(session_id)
        if not local: continue
        totals.update(local); samples.extend(token_rows); sessions.append({'session':digest(str(session_id or path.name)),'calls':local['calls'],'input_tokens':local['input_tokens'],'requests':len(token_rows),'first_input':token_rows[0] if token_rows else None,'last_input':token_rows[-1] if token_rows else None,'max_input':max(token_rows,default=0)})
    recommendations=[]
    for cat,(pattern,destination) in RULES.items():
        if repeat[cat] or large[cat] or failures[cat]: recommendations.append({'category':cat,'destination':destination,'calls':groups[cat],'exact_repeat_candidates':repeat[cat],'large_output_candidates':large[cat],'failure_candidates':failures[cat],'review_priority':repeat[cat]+2*large[cat]+failures[cat]})
    recommendations.sort(key=lambda x:-x['review_priority'])
    return {'sessions':len(sessions),'totals':dict(totals),'request_input':{'count':len(samples),'mean':round(sum(samples)/len(samples)) if samples else None,'p50':quantile(samples,.5),'p95':quantile(samples,.95),'max':max(samples,default=None)},'tool_categories':dict(groups),'correction_candidates':dict(corrections),'correction_references':dict(examples),'recommendations':recommendations,'largest_sessions':sorted(sessions,key=lambda x:-x['input_tokens'])[:20],'event_types':dict(events),'limitations':['Exact repeated commands may be legitimate after source changes; no causal attribution.','Categories overlap; counts must not be summed as unique events.','User correction and failure detection are regex heuristics.','Usage uses last_token_usage on distinct consecutive cumulative counters; forks, resets and missing events can distort totals.','No raw message or command text is exported; references use hashed session IDs and line numbers.','No automatic AGENTS.md or tooling changes.']}

def command_text(raw):
    if isinstance(raw, str):
        try: raw=json.loads(raw)
        except ValueError: return raw
    if isinstance(raw, dict):
        for key in ('cmd','command','script','input'):
            if key in raw: return command_text(raw[key])
    if isinstance(raw,list): return ' '.join(map(str,raw))
    return str(raw)

def detailed(paths, repo, since, until=None):
    import shlex
    files=C.defaultdict(lambda: {'reads':0,'sessions':set(),'overlaps':0,'references':[]})
    searches=C.defaultdict(lambda: {'calls':0,'sessions':set()})
    tools=C.Counter(); manual=C.Counter(); repeats=C.Counter(); corrections=C.Counter(); examples=[]; largest=[]; contexts=[]; seen=set(); errors=0
    root=Path(repo).resolve() if repo else None
    for path in paths:
        sid=None; cwd=None; eligible=False; pending={}; ranges=C.defaultdict(list); tests=set(); epoch=0; usage=[]; compact=[]; previous=None; edited=set(); last_sig=None
        try:
            with path.open(errors='replace') as stream:
                for n,line in enumerate(stream,1):
                    try: row=json.loads(line); p=row.get('payload',{}); typ=row.get('type'); pt=p.get('type')
                    except (ValueError,AttributeError): errors+=1; continue
                    if typ=='session_meta':
                        sid=str(p.get('id') or path.name); cwd=Path(p.get('cwd') or '/').resolve()
                        if sid in seen or (root and not cwd.is_relative_to(root)): break
                        seen.add(sid); eligible=True
                    if not eligible: continue
                    day=str(row.get('timestamp',''))[:10]
                    if not day or (since and day<since) or (until and day>=until): continue
                    ref={'session':digest(sid),'line':n}
                    if typ=='compacted' or pt in ('context_compacted','compaction'):
                        compact.append(len(usage))
                    if typ=='event_msg' and pt=='token_count':
                        info=p.get('info') or {}; u=info.get('last_token_usage') or {}; sig=info.get('total_token_usage'); v=u.get('input_tokens')
                        if sig and sig!=last_sig and isinstance(v,int) and v>=0: usage.append(v); last_sig=sig
                    if typ=='response_item' and pt=='message' and p.get('role')=='user':
                        for k,pattern in CORRECTIONS.items():
                            if re.search(pattern,content(p),re.I):
                                corrections[k]+=1
                                if len(examples)<30: examples.append(dict(ref,category=k,previous_tool=previous))
                    if typ!='response_item': continue
                    if pt in ('function_call','custom_tool_call'):
                        name=str(p.get('name','')); raw=p.get('arguments',p.get('input',''))
                        try: a=json.loads(raw) if isinstance(raw,str) else raw
                        except ValueError: a={}
                        if not isinstance(a,dict): a={}
                        cmd=command_text(raw); previous=digest(cmd); work=Path(a.get('workdir') or cwd).resolve(); call=str(p.get('call_id','')); base=root or cwd
                        label=name; targets=[]
                        # Lexical extraction only: never execute historical shell text.
                        # Handle compound commands, wrappers and bounded excerpts.
                        try:
                            lexer=shlex.shlex(cmd,posix=True,punctuation_chars=';&|')
                            lexer.whitespace_split=True; lexer.commenters=''
                            tokens=list(lexer)
                        except ValueError: tokens=[]
                        segments=[[]]
                        for token in tokens:
                            if token and all(c in ';&|' for c in token): segments.append([])
                            else: segments[-1].append(token)
                        for tokens in segments:
                            if not tokens: continue
                            while tokens and (tokens[0] in ('rtk','command','env','time','sudo') or re.match(r'^\w+=',tokens[0])): tokens=tokens[1:]
                            if not tokens: continue
                            op=Path(tokens[0]).name
                            if op=='cd' and len(tokens)==2 and '$' not in tokens[1]:
                                work=(work/tokens[1]).resolve(); continue
                            label=op
                            if op in ('cat','sed','head','tail','read','bulk-read','nl','bat','awk'):
                                span=None
                                m=re.search(r'\b(\d+),(\d+)p\b',' '.join(tokens))
                                if m: span=tuple(map(int,m.groups()))
                                for token in tokens[1:]:
                                    if token.startswith('-') or '$' in token: continue
                                    candidate=(work/token).resolve()
                                    if base and candidate.is_relative_to(base) and candidate.suffix in ('.rs','.ts','.tsx','.py','.md','.toml','.sql','.json','.yaml','.yml','.js','.cjs','.sh'):
                                        key=str(candidate.relative_to(base)); targets.append(key); f=files[key]; f['reads']+=1; f['sessions'].add(sid)
                                        if len(f['references'])<5: f['references'].append(ref)
                                        if span and any(max(span[0],x[0])<=min(span[1],x[1]) for x in ranges[key]): f['overlaps']+=1
                                        if span: ranges[key].append(span)
                            if op in ('rg','grep','ast-grep'):
                                key=digest(' '.join(tokens[1:])); searches[key]['calls']+=1; searches[key]['sessions'].add(sid)
                            if re.match(r'(just (test|check|typecheck)|cargo (test|check))\b',' '.join(tokens)):
                                key=(epoch,digest(' '.join(tokens)))
                                if key in tests: repeats['same_test_no_detected_edit']+=1
                                tests.add(key)
                        if name=='apply_patch' or re.search(r'\b(write_text|write_bytes|sed -i|git checkout|git restore|cargo fmt|just fmt)\b|(?<![<])>(?![>])',cmd): epoch+=1; ranges.clear()
                        for m in re.finditer(r'(?:scripts/(?:local|agent|diagnostics|api|db)/|\.codex/)[\w./-]+\.(?:py|cjs|sh|js)',cmd): tools[m.group()]+=1
                        if re.search(r'\b(sqlite3|curl|python3?\s+-)\b',cmd) and not re.search(r'scripts/(local|agent|diagnostics|api|db)/',cmd): manual[label]+=1
                        pending[call]=(label,ref,targets,previous)
                    elif pt in ('function_call_output','custom_tool_call_output'):
                        out=p.get('output',''); size=len(out) if isinstance(out,str) else len(json.dumps(out)); item=pending.pop(str(p.get('call_id','')),None)
                        if item and size>20000:
                            largest.append(dict(item[1],tool=item[0],characters=size,files=item[2],command_hash=item[3])); largest=sorted(largest,key=lambda x:-x['characters'])[:30]
            if eligible and usage:
                contexts.append({'session':digest(sid),'requests':len(usage),'mean':round(sum(usage)/len(usage)),'first':usage[0],'last':usage[-1],'max':max(usage),'compaction_pairs':[{'before':usage[i-1],'after':usage[i]} for i in compact if 0<i<len(usage)]})
        except OSError: errors+=1
    nav=[dict(v,file=k,sessions=len(v['sessions'])) for k,v in files.items()]; nav.sort(key=lambda x:(-x['sessions'],-x['reads']))
    return {'navigation':nav[:40],'searches':[{'hash':k,'calls':v['calls'],'sessions':len(v['sessions'])} for k,v in sorted(searches.items(),key=lambda x:-x[1]['calls'])[:30]],'context':sorted(contexts,key=lambda x:-x['max'])[:30],'corrections':dict(corrections),'correction_examples':examples,'tooling':dict(tools.most_common(40)),'manual_tool_candidates':dict(manual),'test_repeat_candidates':dict(repeats),'largest_outputs':largest,'parse_or_io_errors':errors,'recommendations':[{'destination':'Beads','proposal':'Verify and remember reusable symbols and responsibilities for '+x['file'],'evidence':x['references'],'sessions':x['sessions']} for x in nav[:15] if x['sessions']>=2], 'limitations':['Lexical shell parsing supports common compound commands; wrappers, pipelines, heredocs and edits may still be missed. Paths are repository-relative candidates, not verified historical files.','Test repeats are candidates, not proof of unchanged inputs. Manual calls are not proof of tooling bypass.','Correction matching is lexical, not semantic. Compaction pairs are adjacent observed usage events, not causal measurements.','No raw commands or messages exported. Repository filenames are included.']}

def main():
    ap=argparse.ArgumentParser(description=__doc__); ap.add_argument('--days',type=int,default=30); ap.add_argument('--all',action='store_true'); ap.add_argument('--repo',default=None,help='Exact session cwd prefix, e.g. /home/ronnie/dev/sbe_ts'); ap.add_argument('--root',action='append',type=Path,help='Additional or alternative session directories'); ap.add_argument('--output',type=Path,default=Path.home()/'.cache/sbe-diagnostics/session-review'); ap.add_argument('--navigation',action='store_true'); ap.add_argument('--context',action='store_true'); ap.add_argument('--corrections',action='store_true'); ap.add_argument('--tooling',action='store_true'); ap.add_argument('--recommendations',action='store_true'); ap.add_argument('--compare',type=Path,help='Previous v2 JSON report'); args=ap.parse_args()
    if args.days<1: ap.error('--days must be positive')
    if args.repo:
        r=Path(args.repo).expanduser().resolve()
        for candidate in (r,*r.parents):
            if (candidate/'.git').exists(): r=candidate; break
        args.repo=str(r)
    roots=args.root or [Path.home()/'.codex/sessions',Path.home()/'.codex/archived_sessions']; paths=sorted({p.resolve() for r in roots for p in r.expanduser().rglob('*.jsonl') if p.is_file()},key=lambda p:p.stat().st_mtime,reverse=True); since=None if args.all else (D.datetime.now(D.timezone.utc)-D.timedelta(days=args.days)).date().isoformat(); report=audit(paths,since,args.repo); report.update({'since_utc_date':since,'files_discovered':len(paths),'repo_filter':args.repo,'status':'REVIEW_REQUIRED' if report['sessions'] else 'NO_MATCHING_DATA'})
    report['schema_version']=3; report['behaviour']=detailed(paths,args.repo,since)
    if args.compare:
        old=json.loads(args.compare.read_text()); report['comparison']={'same_repo_filter':old.get('repo_filter')==args.repo,'previous_since':old.get('since_utc_date'),'current_since':since,'metrics':{k:{'previous':old.get('request_input',{}).get(k),'current':report['request_input'].get(k)} for k in ('count','mean','p50','p95','max')},'note':'Periods may overlap; workload and schema differences prevent causal attribution.'}
    args.output.mkdir(parents=True,exist_ok=True,mode=0o700)
    with tempfile.NamedTemporaryFile(mode='w',dir=args.output,prefix='review-',suffix='.json',delete=False) as f: json.dump(report,f,indent=2); out=f.name
    print(json.dumps({k:v for k,v in report.items() if k not in ('event_types','largest_sessions','correction_references','behaviour')},indent=2)); print('REPORT:',out)
    selected=[k for k in ('navigation','context','corrections','tooling','recommendations') if getattr(args,k)]
    if report['sessions'] and not report['behaviour']['navigation'] and report['tool_categories'].get('read',0): print('WARNING: read commands detected but no repository paths extracted; inspect command schema locally.')
    for k in selected or ['recommendations']:
        print(k.upper(),json.dumps(report['behaviour'][k],indent=2))
if __name__=='__main__': main()
