import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import path from 'node:path';
const dir = path.resolve(process.env.ROWSCAN_DFS_WASM_DIR || 'wasm/target/wasm32-unknown-emscripten/release/deps');
const {default:createSolver} = await import(pathToFileURL(path.join(dir,'dfs_solver.js')));
const module = await createSolver({wasmBinary:await readFile(path.join(dir,'dfs_solver.wasm'))});
const encoder = new TextEncoder();
function call(name, url='', options={}) {
  const buffers = [encoder.encode(url), encoder.encode(typeof options==='string'?options:JSON.stringify(options))];
  const ptrs = buffers.map(bytes => {const ptr=module._malloc(Math.max(1,bytes.length));module.HEAPU8.set(bytes,ptr);return ptr;});
  try {
    const ptr = name==='dfs_resume' ? module._dfs_resume() : module[`_${name}`](ptrs[0],buffers[0].length,ptrs[1],buffers[1].length);
    const heap = module.HEAPU8;
    const length = new DataView(heap.buffer).getUint32(ptr,true);
    return JSON.parse(new TextDecoder().decode(heap.subarray(ptr+4,ptr+4+length)));
  } finally {ptrs.forEach(ptr => module._free(ptr));}
}
const samples = Object.fromEntries((await readFile('tests/fixtures/urls.txt','utf8')).trim().split('\n').map(l=>l.split(/\s+/)));

test('exports, prepare label map, defaults, glyph orientation and no-solution replies',()=>{
  for(const name of ['dfs_prepare','dfs_solve','dfs_resume','malloc','free']) assert.equal(typeof module[`_${name}`],'function');
  const prep=call('dfs_prepare','p?numlin/2/2/+abc-10+abc-10');
  assert.deepEqual(prep.value_map,[16,2748]);
  assert.equal(prep.width,2);assert.equal(prep.height,2);
  const defaults=call('dfs_solve','p?numlin/3/1/1g1');
  assert.equal(defaults.memo_slots,2**24);
  assert.deepEqual(defaults.answer,{width:3,height:1,links:[2,10,8],has_empty:false});
  assert.equal(defaults.first_has_empty,false);
  assert.equal(defaults.first_solution_has_empty,false);
  const none=call('dfs_solve','p?numlin/2/2/1221',{memo_bits:0});
  assert.equal(none.solutions,0);assert.equal(none.answer,null);assert.equal(none.first_has_empty,null);
});

test('budget and continuation preserve all cumulative counters and first paths, all directions',()=>{
  let first;
  for(let direction=0;direction<8;direction++) for(const memo_bits of [0,12]) {
    const opts={direction,cut:25,memo_bits};
    const probe=call('dfs_solve',samples.ex3,{...opts,race_budget:10});
    assert.equal(probe.steps,10);assert.equal(probe.finished,false);
    assert.ok(probe.progress>=0&&probe.progress<=1);
    assert.ok(probe.est_total===null||Number.isFinite(probe.est_total));
    const resumed=call('dfs_resume');
    assert.equal(resumed.finished,true);assert.equal(resumed.progress,1);
    const fresh=call('dfs_solve',samples.ex3,opts);
    for(const key of ['steps','solutions','memo_hits','memo_stores','memo_slots','answer','first_has_empty']) assert.deepEqual(resumed[key],fresh[key],key);
    assert.equal(resumed.solutions,1);
    if(first) assert.deepEqual(resumed.answer,first); else first=resumed.answer;
    assert.match(call('dfs_resume').error,/no saved search/);
  }
});

test('replacement, automatic budget, capped search, malformed inputs and recoverability',()=>{
  const probe=call('dfs_solve',samples.ex3,{memo_bits:0,race_budget:1});
  assert.equal(probe.race_budget,2*225**2);
  call('dfs_solve','p?numlin/3/1/1g1',{memo_bits:0,max:1});
  assert.match(call('dfs_resume').error,/no saved search/);
  for(const options of ['[1]','null','{',{memo_bits:29},{memo_bits:-1},{direction:8},{direction:'wrong'},{diag_shortcut:1},{cut:-1},{max:1.5},{race_budget:-1}]) {
    assert.ok(call('dfs_solve','p?numlin/3/1/1g1',options).error);
  }
  assert.ok(call('dfs_prepare','p?numlin/2/2/1').error);
  assert.equal(call('dfs_solve','p?numlin/3/1/1g1',{memo_bits:0}).solutions,1);
});


test('direct race selects and resumes a retained candidate',()=>{
  const partial=call('dfs_solve',samples.ex3,{direction:'race',memo_bits:0,cut:25,race_budget:10});
  assert.equal(partial.finished,false);
  const resumed=call('dfs_resume');
  const fresh=call('dfs_solve',samples.ex3,{direction:partial.direction,memo_bits:0,cut:25});
  for(const key of ['steps','solutions','memo_hits','memo_stores','answer']) assert.deepEqual(resumed[key],fresh[key],key);
  const finished=call('dfs_solve','p?numlin/3/1/1g1',{direction:'race',memo_bits:0});
  assert.equal(finished.finished,true);assert.equal(finished.solutions,1);
});


test('wide pair masks, real G1/G2 work and original orientation',async()=>{
  const rows=(await readFile('reference/ex3.txt','utf8')).trim().split('\n').slice(1,-1);
  for(const pairs of [63,87]) {
    const width=pairs===63?23:25;
    const cells=[];let next=13;
    for(let y=0;y<15;y++) {
      cells.push(...[...rows[y]].map(c=>c==='.'?0:parseInt(c,36)));
      const extra=pairs===63?(y<6?4:3):5;
      for(let i=0;i<extra;i++) {cells.push(next,next);next++;}
      while(cells.length<(y+1)*width)cells.push(0);
    }
    const hint=n=>n===0?'g':n<16?n.toString(16):'-'+n.toString(16).padStart(2,'0');
    const url=`p?numlin/${width}/15/${cells.map(hint).join('')}`;
    for(let direction=0;direction<8;direction++) {
      const opts={direction,cut:25,memo_bits:12};
      const result=call('dfs_solve',url,opts);
      assert.equal(result.solutions,1);assert.equal(result.answer.links.length,width*15);
    }
  }
});
