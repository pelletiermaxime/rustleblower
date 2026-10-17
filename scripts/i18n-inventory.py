#!/usr/bin/env python3
"""Conservative migration inventory; literal candidates need human classification.
No source/catalogue rewrite. --check requires complete shipped key/placeholder coverage;
--require-complete additionally rejects remaining direct UI literal candidates.
"""
import argparse,json,pathlib,re,sys
ROOT=pathlib.Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser();p.add_argument('--output',type=pathlib.Path);p.add_argument('--check',action='store_true');p.add_argument('--require-complete',action='store_true');a=p.parse_args()
errors=[];catalogues={}
def is_ui_test(path):
 return path.name == 'kit_tests.rs' or 'kit_tests' in path.parts
def pairs(items):
 out={}
 for k,v in items:
  if k in out:raise ValueError('duplicate key '+k)
  out[k]=v
 return out
for path in sorted((ROOT/'locales').glob('*.json')):
 try:catalogues[path.stem]=json.loads(path.read_text(),object_pairs_hook=pairs)['messages']
 except (ValueError,KeyError) as e:errors.append(str(e))
en=catalogues.get('en',{});names=lambda s:set(re.findall(r'%\{([A-Za-z0-9_]+)\}',s))
coverage={}
for locale,messages in catalogues.items():
 missing=sorted(set(en)-set(messages));extra=sorted(set(messages)-set(en));bad=sorted(k for k in set(messages)&set(en) if names(messages[k])!=names(en[k]))
 coverage[locale]={'translated':len(messages),'total':len(en),'missing':missing,'unknown':extra,'placeholder_mismatch':bad}
 if missing or extra or bad:errors.append(locale+' catalogue coverage or placeholders differ')
uses=[];candidates=[];literal_counts={}
source_keys={value:key for key,value in en.items()}
for path in sorted((ROOT/'src').rglob('*.rs')):
 if is_ui_test(path):continue
 text=path.read_text();relative=str(path.relative_to(ROOT))
 literal_counts[relative]=len(re.findall(r'"(?:[^"\\]|\\.)*"',text))
 for m in re.finditer(r'(?:i18n|crate::i18n)::(?:text|format)\(\s*"([^"]+)"',text):
  key=m.group(1);uses.append({'file':relative,'line':text.count('\n',0,m.start())+1,'key':key})
  if key not in en:errors.append(relative+': unknown translation key '+key)
 for m in re.finditer(r'(?:i18n|crate::i18n)::(?:tr|tr_args|placeholder)\(\s*("(?:[^"\\]|\\.)*")',text):
  try:source=json.loads(m.group(1))
  except ValueError:continue
  key=source_keys.get(source);uses.append({'file':relative,'line':text.count('\n',0,m.start())+1,'key':key,'source':source})
  if key is None:errors.append(relative+': uncatalogued source presentation string '+source)
 # Conditional display branches are also source strings. Keep this conservative:
 # only direct literal arms of one if/else; never infer prose from comparisons,
 # enum keys, reported data, or arbitrary expressions in the condition.
 conditional=r'(?:i18n|crate::i18n)::(?:tr|tr_args)\(\s*if\b(?:[^{}"]|"(?:[^"\\]|\\.)*")*\{\s*("(?:[^"\\]|\\.)*")\s*\}\s*else\s*\{\s*("(?:[^"\\]|\\.)*")\s*\}'
 for m in re.finditer(conditional,text):
  for literal in m.groups():
   try:source=json.loads(literal)
   except ValueError:continue
   key=source_keys.get(source);uses.append({'file':relative,'line':text.count('\n',0,m.start())+1,'key':key,'source':source,'conditional':True})
   if key is None:errors.append(relative+': uncatalogued conditional presentation string '+source)
 if relative.startswith('src/app'):
  for m in re.finditer(r'\.(label|placeholder|description|tooltip|title|child|item|accessibility_label)\(\s*(format!\(\s*)?("(?:[^"\\]|\\.)*")',text):
   try:value=json.loads(m.group(3))
   except ValueError:continue
   prose=re.sub(r'\{[^}]*\}','',value) if m.group(2) else value
   if value in ('argv[{index}]','Rustleblower'):continue # Argument notation and the product name, not translatable prose.
   if prose and any(c.isalpha() for c in prose):candidates.append({'file':relative,'line':text.count('\n',0,m.start())+1,'api':m.group(1),'literal':value})
# Separate conservative categories: these are review queues, not automatic
# translation targets. Commands, paths, protocol data and IDs must stay stable.
status_literals=[];app_format_literals=[];core_validation_literals=[];metadata_literals=[]
quoted=r'("(?:[^"\\]|\\.)*")'
for path in sorted((ROOT/'src').rglob('*.rs')):
 if is_ui_test(path):continue
 source=path.read_text();relative=str(path.relative_to(ROOT));is_ui=relative.startswith('src/app')
 categories=[]
 if is_ui:
  categories.append((status_literals,r'(?:\.error|\.message)\s*=\s*Some\(\s*'+quoted))
  categories.append((app_format_literals,r'format!\(\s*'+quoted))
 else:
  categories.append((core_validation_literals,r'(?:bail!|anyhow!|\.context|\.ok_or)\(\s*'+quoted))
  categories.append((core_validation_literals,r'Err\(\s*'+quoted))
  categories.append((metadata_literals,r'(?:label|hint):\s*'+quoted))
 for category,pattern in categories:
  for match in re.finditer(pattern,source):
   try:value=json.loads(match.group(1))
   except ValueError:continue
   if not value or not any(c.isalpha() for c in value):continue
   # Pure positional formatting is data composition, without prose to translate.
   if category is app_format_literals and not re.search(r'[A-Za-z]{2}',re.sub(r'\{[^}]*\}','',value)):continue
   category.append({'file':relative,'line':source.count('\n',0,match.start())+1,'literal':value,'catalogued':value in source_keys})
# ensure! diagnostics are commonly multiline; record the final string in the
# bounded macro statement as a separate conservative review queue. This does
# not rewrite control strings or assume every diagnostic reaches a view.
ensure_literals=[]
for path in sorted((ROOT/'src').rglob('*.rs')):
 if is_ui_test(path) or path.name == 'tests.rs':continue
 source=path.read_text();relative=str(path.relative_to(ROOT))
 for match in re.finditer(r'ensure!\(([\s\S]*?)\);',source):
  values=[]
  for literal in re.finditer(quoted,match.group(1)):
   try:values.append(json.loads(literal.group(1)))
   except ValueError:pass
  if values and any(c.isalpha() for c in values[-1]):
   value=values[-1]
   ensure_literals.append({'file':relative,'line':source.count('\n',0,match.start())+1,'literal':value,'catalogued':value in source_keys,'dynamic':bool(re.search(r'\{[^}]*\}',value))})
unclassified_ui=[]
for path in sorted((ROOT/'src/app').rglob('*.rs')):
 if is_ui_test(path):continue
 source=path.read_text();relative=str(path.relative_to(ROOT))
 for match in re.finditer(quoted,source):
  try:value=json.loads(match.group(1))
  except ValueError:continue
  prose=re.sub(r'\{[^}]*\}','',value)
  if value in source_keys or value.startswith(('/', 'http:', 'https:', 'dev.rustleblower.')):continue
  if not re.search(r'[A-Za-z]{3}',prose):continue
  if not any(c.isspace() for c in prose) and not prose[:1].isupper():continue
  unclassified_ui.append({'file':relative,'line':source.count('\n',0,match.start())+1,'literal':value})
result={'schema':1,'scope':'Static key coverage is exact; literal candidates are a conservative migration inventory, not semantic full-UI coverage. Separate format/status/core/metadata queues require manual classification; internal identifiers are not translation targets.','catalogues':coverage,'kit_keys':sum(k.startswith('gpui_component.') for k in en),'static_uses':uses,'direct_ui_literal_candidates':candidates,'source_literal_counts':literal_counts,'status_literal_candidates':status_literals,'app_format_literal_candidates':app_format_literals,'core_validation_literal_candidates':core_validation_literals,'metadata_label_hint_candidates':metadata_literals,'ensure_validation_literal_candidates':ensure_literals,'unclassified_ui_literal_candidates':unclassified_ui,'errors':errors}
output=json.dumps(result,ensure_ascii=False,indent=2)+'\n'
if a.output:a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(output)
else:print(output,end='')
print(f"i18n: {len(en)} English keys, {len(uses)} static uses, {len(candidates)} direct UI literal candidates, {len(errors)} errors",file=sys.stderr)
if (a.check and errors) or (a.require_complete and (errors or candidates)):sys.exit(1)
