// node check-keys.mjs -> exits 1 if any pair (zh-CN/en, pro.zh-CN/pro.en) differs in keys or {{placeholders}}, or a key exists in both free and pro files
import fs from "node:fs";
const flat=(o,p="")=>Object.entries(o).flatMap(([k,v])=>typeof v==="object"?flat(v,p+k+"."):[[p+k,v]]);
const load=f=>Object.fromEntries(flat(JSON.parse(fs.readFileSync(f,"utf8"))));
const ph=s=>[...new Set(s.match(/\{\{\w+\}\}/g)||[])].sort().join();
let errs=[],out=[];
for(const [za,eb,tag] of [["zh-CN.json","en.json","free"],["pro.zh-CN.json","pro.en.json","pro"]]){
 const a=load(za),b=load(eb);
 errs.push(...Object.keys(a).filter(k=>!(k in b)).map(k=>`[${tag}] missing en: ${k}`),...Object.keys(b).filter(k=>!(k in a)).map(k=>`[${tag}] missing zh-CN: ${k}`),...Object.keys(a).filter(k=>k in b&&ph(a[k])!==ph(b[k])).map(k=>`[${tag}] placeholder: ${k}`));
 out.push(`${tag} ${Object.keys(a).length}`);}
const f=load("zh-CN.json"),p=load("pro.zh-CN.json");errs.push(...Object.keys(f).filter(k=>k in p).map(k=>"in both free and pro: "+k));
console.log(errs.length?errs.join("\n"):"OK "+out.join(" + ")+" keys");process.exit(errs.length?1:0);
