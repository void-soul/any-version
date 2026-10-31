from pathlib import Path
import re, subprocess, tempfile, json
source=Path('src-tauri/src/commands/cert/deploy_ssh_transaction.rs').read_text(encoding='utf8')
template=re.search(r'r#"(set -eu.*?)"#',source,re.S).group(1)
def quote(v): return "'"+str(v).replace("'", "'\\''")+"'"
def posix(path):
 p=Path(path).resolve();return '/'+p.drive[0].lower()+p.as_posix()[2:]
cases=[('success','cp -- "$c" "$runtime"',False,False),('mismatch',':',True,False),('second_move',':',False,True),('partial_reload','cp -- "$c" "$runtime"; if [ "$(cat -- "$c")" = new-pair ]; then false; fi',False,False),('recovery_failure','cp -- "$c" "$runtime"; false',False,False),('errexit','false; true',False,False)]
for name,post,mismatch,fail_move in cases:
 with tempfile.TemporaryDirectory(prefix='cert-ssh-fake-') as temporary:
  root=Path(temporary).resolve();assert root.parent==Path(tempfile.gettempdir()).resolve()
  cert=root/'cert dir'/'fullchain.pem';key=root/"key'dir"/'private.key';cert.parent.mkdir();key.parent.mkdir()
  cert.write_text('old-cert');key.write_text('old-key');runtime=root/'runtime';runtime.write_text('old-cert')
  Path(str(cert)+'.kira-test.new').write_text('new-pair');Path(str(key)+'.kira-test.new').write_text('different' if mismatch else 'new-pair')
  (root/'openssl').write_text('#!/bin/sh\ncat -- "$3"\n',encoding='utf8')
  if fail_move:(root/'mv').write_text('#!/bin/sh\ncase "$3" in *private.key.kira-test.new) exit 9;; esac\nexec /bin/mv "$@"\n',encoding='utf8')
  realpost='runtime='+quote(posix(runtime))+'; '+post
  values=dict(c=quote(posix(cert)),k=quote(posix(key)),cs=quote(posix(str(cert)+'.kira-test.new')),ks=quote(posix(str(key)+'.kira-test.new')),cb=quote(posix(str(cert)+'.kira-test.old')),kb=quote(posix(str(key)+'.kira-test.old')),post=realpost,post_script=quote(realpost))
  scriptfile=root/'transaction.sh';scriptfile.write_text(template.format(**values),encoding='utf8',newline='\n')
  command='export PATH='+quote(posix(root))+':$PATH; chmod 700 '+quote(posix(root/'openssl'))+'; '+('chmod 700 '+quote(posix(root/'mv'))+'; ' if fail_move else '')+'sh '+quote(posix(scriptfile))
  result=subprocess.run(['C:/Program Files/Git/bin/bash.exe','--noprofile','--norc','-c',command],capture_output=True,encoding='utf8',timeout=10)
  confirmed='KIRA_ROLLBACK_CONFIRMED' in result.stderr.splitlines()
  retained=Path(str(cert)+'.kira-test.old').exists() and Path(str(cert)+'.kira-lock').exists()
  state=(cert.read_text(),key.read_text(),runtime.read_text())
  if name=='success':assert result.returncode==0 and state==('new-pair','new-pair','new-pair'),(name,result.stderr,state)
  else:
   assert result.returncode!=0 and state[:2]==('old-cert','old-key'),(name,result.stderr,state)
   if name in ['second_move','partial_reload']:assert confirmed and not retained and state[2]=='old-cert',(name,result.stderr,state)
   if name in ['recovery_failure','errexit']:assert not confirmed and retained and 'KIRA_RUNTIME_ROLLBACK_UNCONFIRMED' in result.stderr,(name,result.stderr)
  print(json.dumps({'case':name,'passed':True,'exit':result.returncode,'runtime':state[2],'rollback_confirmed':confirmed,'backups_retained':retained}))
