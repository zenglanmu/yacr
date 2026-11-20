#!/usr/bin/env python3
"""Materialise the importer's synthetic textured IFC4 fixture as a real file.

One `IfcTriangulatedFaceSet` quad (1 m, metres declared) carrying an
`IfcIndexedTriangleTextureMap` whose `IfcPixelTexture` is two RGB pixels
(red, green) — i.e. a texture whose bytes live inside the IFC, no external file.
"""
import sys

BODY = """ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('ViewDefinition [CoordinationView]'),'2;1');
FILE_NAME('synthetic-textured-tessellation.ifc','2026-11-20T00:00:00',('yacr'),('yacr'),'yacr synthetic fixture','yacr','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('2O2Fr$t4X7Zf8NOew3FL00',$,'Textured project',$,$,$,$,(#2),#9);
#2=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#3,$);
#3=IFCAXIS2PLACEMENT3D(#4,#5,#6);
#4=IFCCARTESIANPOINT((0.,0.,0.));
#5=IFCDIRECTION((0.,0.,1.));
#6=IFCDIRECTION((1.,0.,0.));
#7=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#8=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);
#9=IFCUNITASSIGNMENT((#7,#8));
#10=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(1.,0.,0.),(1.,1.,0.),(0.,1.,0.)),$);
#11=IFCTRIANGULATEDFACESET(#10,$,.T.,((1,2,3),(1,3,4)),$);
#12=IFCSHAPEREPRESENTATION(#2,'Body','Tessellation',(#11));
#13=IFCPRODUCTDEFINITIONSHAPE($,$,(#12));
#14=IFCLOCALPLACEMENT($,#15);
#15=IFCAXIS2PLACEMENT3D(#16,$,$);
#16=IFCCARTESIANPOINT((0.,0.,0.));
#17=IFCBUILDINGELEMENTPROXY('2O2Fr$t4X7Zf8NOew3FL01',$,'Panel',$,$,#14,#13,$,$);
#20=IFCPIXELTEXTURE(.T.,.T.,'MODULATE',$,$,2,1,3,("0FF0000","000FF00"));
#21=IFCTEXTUREVERTEXLIST(((0.,0.),(1.,0.),(1.,1.),(0.,1.)));
#22=IFCINDEXEDTRIANGLETEXTUREMAP((#20),#11,#21,((1,2,3),(1,3,4)));
ENDSEC;
END-ISO-10303-21;
"""

if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else "."
    with open(f"{target}/checker-embedded-texture.ifc", "w") as f:
        f.write(BODY)
    print("wrote checker-embedded-texture.ifc")
