"""Regressions for diagram identity and route geometry."""
import copy
import tempfile
import math
import unittest
from pathlib import Path
from unittest.mock import patch

import diagrams
import check_diagrams


class DiagramInjectionTests(unittest.TestCase):
    ids = ['d-cluster-local', 'd-console']

    def block(self, id):
        return f'<h3 id="{id}">{id}</h3>\n<!-- diagram:{id} -->\nold SVG\n<!-- /diagram:{id} -->'

    def inject(self, content, rejected=False):
        with tempfile.TemporaryDirectory() as directory:
            page = Path(directory, 'example.html')
            page.write_text(content)
            with patch.object(diagrams, '__file__', str(Path(directory, 'diagrams.py'))):
                if rejected:
                    with self.assertRaises(AssertionError):
                        diagrams.inject(page.name, self.ids)
                    self.assertEqual(page.read_text(), content)
                else:
                    diagrams.inject(page.name, self.ids)
                    result = page.read_text()
                    diagrams.inject(page.name, self.ids)
                    self.assertEqual(page.read_text(), result)
                    return result

    def test_reordered_sections_keep_their_diagram_identity(self):
        order = list(reversed(self.ids))
        result = self.inject('\n'.join(self.block(id) for id in order))
        self.assertEqual([m.group(1) for m in diagrams.BLOCK.finditer(result)], order)
        for id in order:
            self.assertIn(f'<h3 id="{id}">{id}</h3>\n<!-- diagram:{id} -->\n'
                          + diagrams.DIAGRAMS[id].render(), result)

    def test_duplicate_id_is_rejected_before_writing(self):
        self.inject('\n'.join([self.block(self.ids[0])] * 2), rejected=True)

    def test_missing_id_is_rejected_before_writing(self):
        self.inject(self.block(self.ids[0]), rejected=True)

    def test_unknown_id_is_rejected_before_writing(self):
        self.inject(self.block(self.ids[0]) + '\n' + self.block('unknown'), rejected=True)

    def test_mixed_legacy_and_marked_blocks_are_rejected(self):
        legacy = '<pre class="mermaid">\nold diagram</pre>'
        self.inject(self.block(self.ids[0]) + '\n' + legacy, rejected=True)

    def test_ha_service_ports_touch_the_capsule_outline(self):
        diagram = diagrams.DIAGRAMS['d-cluster-ha']
        for edge in diagram.edges:
            for node_id, face in [(edge['a'], edge['fa']), (edge['b'], edge['fb'])]:
                node = diagram.nodes[node_id]
                if node['kind'] != 'svc':
                    continue
                x, y, _ = diagram.anchor(node_id, face)
                dx, dy = x - node['x'], y - node['y']
                radius = node['h'] / 2
                half_flat = node['w'] / 2 - radius
                with self.subTest(node=node_id, face=face):
                    if abs(dx) <= half_flat:
                        self.assertAlmostEqual(abs(dy), radius)
                    else:
                        center_x = math.copysign(half_flat, dx)
                        self.assertAlmostEqual((dx - center_x) ** 2 + dy ** 2, radius ** 2)


class DiagramRouteTests(unittest.TestCase):
    def test_container_detached_source_is_rejected(self):
        diagram = copy.deepcopy(diagrams.DIAGRAMS['d-docker-cluster'])
        edge = next(e for e in diagram.edges if e['a'] == 'AGT' and e['b'] == 'RAG')
        edge['path'] = 'M494,330 L700,330 Q730,330 730,300 L730,258 Q730,240 748,240 L790,240'
        problems = check_diagrams.check_routes(diagram)
        self.assertTrue(any(p[1] == 'начало вне объявленного порта' and p[2] == 'AGT → RAG' for p in problems))

    def test_container_detached_label_is_rejected(self):
        diagram = copy.deepcopy(diagrams.DIAGRAMS['d-docker-cluster'])
        edge = next(e for e in diagram.edges if e['a'] == 'AGT' and e['b'] == 'RAG')
        edge['ly'] = 360
        problems = check_diagrams.check_routes(diagram)
        self.assertTrue(any(p[1] == 'подпись оторвана от своего ребра' and p[2] == 'TCP' for p in problems))

    def test_publication_label_cannot_cover_arrowhead(self):
        diagram = copy.deepcopy(diagrams.DIAGRAMS['d-file-web'])
        edge = next(e for e in diagram.edges if e['a'] == 'WEBI' and e['b'] == 'WS')
        edge['lx'], edge['ly'] = diagram.anchor('WS', edge['fb'])[:2]
        problems = check_diagrams.check_routes(diagram)
        self.assertTrue(any(p[1] == 'подпись закрывает начало или наконечник'
                            and p[3] == 'WEBI → WS' for p in problems))

    def test_local_client_has_direct_and_web_paths(self):
        for diagram_id, target, label in [('d-file-web', 'IB', 'File=…, файл'),
                                          ('d-cluster-web', 'RAG', 'TCP')]:
            with self.subTest(diagram=diagram_id):
                diagram = diagrams.DIAGRAMS[diagram_id]
                edges = {edge['b']: edge for edge in diagram.edges if edge['a'] == 'CLI'}
                self.assertEqual(set(edges), {target, 'WS'})
                self.assertEqual(edges[target]['label'], label)
                self.assertIn('HTTP', edges['WS']['label'])
                self.assertTrue(edges['WS']['dashed'])


    def test_two_publications_keep_the_infobase_identity(self):
        for diagram_id, expected_refs in [('d-web-dual', {'Ref=demo'}),
                                          ('d-web-two-infobases', {'Ref=origin', 'Ref=test'})]:
            with self.subTest(diagram=diagram_id):
                diagram = diagrams.DIAGRAMS[diagram_id]
                targets = [diagram.nodes[e['b']]['text'] for e in diagram.edges
                           if e['a'] in ('PUBLIC', 'INTERNAL') and e['label'] == 'TCP']
                self.assertEqual(len(targets), 2)
                self.assertEqual({text.split('Ref=')[1] for text in targets},
                                 {ref.removeprefix('Ref=') for ref in expected_refs})


if __name__ == '__main__':
    unittest.main()
