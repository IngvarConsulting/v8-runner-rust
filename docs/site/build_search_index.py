"""Build the public site search index from HTML and its existing local producers."""
import html
import json
import re
import subprocess
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parent

# Render only the existing public producers. No network or source-code indexing;
# scenarios.html renders the target-only compute(ctx) model.
RENDER = r"""
const fs = require('fs'), vm = require('vm');
const root = process.argv[1], result = {};
for (const file of ['scenarios.html','usecases.html']) {
  const slots = {}; let selection = '';
  const get = id => slots[id] || (slots[id] = {innerHTML:'', textContent:'', addEventListener(type, handler){this[type]=handler}});
  const context = vm.createContext({window:{}, document:{getElementById:get,querySelector:get}, localStorage:{getItem(){return null},setItem(k,v){selection=v}} });
  for (const asset of ['data.js','usecases.js']) vm.runInContext(fs.readFileSync(root+'/'+asset,'utf8'),context,{timeout:2000});
  const source = fs.readFileSync(root+'/'+file,'utf8');
  for (const match of source.matchAll(/<script>([\s\S]*?)<\/script>/g)) vm.runInContext(match[1],context,{timeout:2000});
  result[file] = [JSON.parse(JSON.stringify(slots))];
  if (file === 'scenarios.html') {
    const axes = context.window.RUNNER_DATA.AXES;
    for (const format of axes.format) for (const type of axes.type) for (const target of axes.target) {
      for (let mask=0; mask < (1 << axes.tools.length); mask++) {
        for (const [name,value] of [['format',format.id],['type',type.id],['target',target.id]]) slots.panel.change({target:{name,value}});
        axes.tools.forEach((tool,i) => slots.panel.change({target:{name:'tool',value:tool.id,checked:!!(mask & (1<<i))}}));
        const copy = JSON.parse(JSON.stringify(slots)); copy.__selection = selection; result[file].push(copy);
      }
    }
  }
}
process.stdout.write(JSON.stringify(result));
"""
rendered = json.loads(subprocess.check_output(["node", "-e", RENDER, str(ROOT)], text=True))


class Sections(HTMLParser):
    def __init__(self, source, filename):
        super().__init__(convert_charrefs=True)
        self.filename = filename
        self.ids = set(re.findall(r'\bid="([^"]+)"', source))
        self.h2_index = 0
        self.in_main = False
        self.heading = None
        self.title = ""
        self.section = ""
        self.anchor = ""
        self.pending_anchor = ""
        self.text = []
        self.records = []

    def flush(self):
        text = " ".join(" ".join(self.text).split())
        if text:
            self.records.append({"url": self.filename + ("#" + self.anchor if self.anchor else ""),
                                 "page": self.title, "section": self.section, "text": text})
        self.text = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "main":
            self.in_main = True
        if not self.in_main:
            return
        if tag == "div" and "uc" in attrs.get("class", "").split():
            self.pending_anchor = attrs.get("id", "")
        if tag in ("h1", "h2", "h3", "h4"):
            self.flush()
            self.heading = []
            if tag == "h2":
                self.h2_index += 1
                anchor = attrs.get("id")
                if not anchor:
                    anchor = f"section-{self.h2_index}"
                    while anchor in self.ids:
                        anchor += "-"
                    self.ids.add(anchor)
                self.anchor = anchor
            elif tag != "h1":
                self.anchor = attrs.get("id") or self.pending_anchor or self.anchor
            self.pending_anchor = ""
        self.text.append(" ")

    def handle_endtag(self, tag):
        if not self.in_main:
            return
        if tag in ("h1", "h2", "h3", "h4") and self.heading is not None:
            name = "".join(self.heading).strip()
            if tag == "h1":
                self.title = name
            self.section = name
            self.heading = None
        if tag == "main":
            self.flush()
            self.in_main = False
        self.text.append(" ")

    def handle_data(self, data):
        if self.in_main:
            self.text.append(data)
            if self.heading is not None:
                self.heading.append(data)


def source_for(filename, slots):
    source = (ROOT / filename).read_text()
    for key, slot in slots.items():
        if key == "__selection":
            continue
        content = slot["innerHTML"] or html.escape(slot["textContent"])
        if key == "#result tbody":
            source = source.replace("<tbody></tbody>", "<tbody>" + content + "</tbody>")
        else:
            pattern = r'(<([a-z]+)\b[^>]*\bid="' + re.escape(key) + r'"[^>]*>).*?(</\2>)'
            source = re.sub(pattern, lambda m: m[1] + content + m[3], source, count=1, flags=re.S)
    return source


# Existing navigation owns page membership and URLs.
navigation = re.search(r"<nav>(.*?)</nav>", (ROOT / "index.html").read_text(), re.S)[1]
pages = list(dict.fromkeys(re.findall(r'href="([\w-]+\.html)(?:#[^"]*)?"', navigation)))
records = []
for filename in pages:
    for slots in rendered.get(filename, [{}]):
        source = source_for(filename, slots)
        parser = Sections(source, filename)
        parser.feed(source)
        if "__selection" in slots:
            from urllib.parse import quote
            query = "?selection=" + quote(slots["__selection"], safe="")
            for record in parser.records:
                base, *fragment = record["url"].split("#")
                record["url"] = base + query + ("#" + fragment[0] if fragment else "")
        records.extend(parser.records)
unique = {}
for record in records:
    key = (record["url"].split("?")[0].split("#")[0], record["section"], record["text"])
    unique.setdefault(key, record)
records = list(unique.values())
output = ROOT / "search-index.json"
output.write_text(json.dumps(records, ensure_ascii=False, indent=2) + "\n")
print(f"Search index: {len(pages)} pages, {len(records)} sections")
