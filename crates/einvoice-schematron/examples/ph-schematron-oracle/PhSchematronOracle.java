// The reference side of the ph-schematron oracle (see main.rs next to this file). Run
// with Java 17 or later in source-file mode, with the class path of
// ph-schematron-maven-plugin 10.3.1 and an SLF4J API:
//
//   java -cp CLASSPATH PhSchematronOracle.java compile OUTDIR FILE.sch...
//   java -cp CLASSPATH PhSchematronOracle.java write OUTDIR FILE.xml...
//   java -cp CLASSPATH PhSchematronOracle.java skeleton OUTDIR
//   java -cp CLASSPATH PhSchematronOracle.java chars OUTFILE
//   java -cp CLASSPATH PhSchematronOracle.java xerces CASES OUTFILE
//
// compile: runs the ISO Schematron pipeline of ph-schematron 10.3.1 (engine
// iso-schematron) on Schematron files, the way the goal `convert` of
// ph-schematron-maven-plugin 10.3.1 does (Schematron2XSLTMojo), and writes, for each
// FILE.sch, into OUTDIR:
//
//   FILE.step1.xml  the result of iso_dsdl_include.xsl, as XML (identity serialization of
//                   the DOM document that step 2 reads)
//   FILE.step2.xml  the result of iso_abstract_expand.xsl, the same way
//   FILE.step3.xml  the result tree of iso_svrl_for_xslt2.xsl, serialized by Saxon
//                   (method xml, no indentation): the input of einvoice-schematron
//   FILE.dom        the DOM document that ph-schematron builds from step 3, node by node
//                   (format below), as ph-schematron's XML writer reads it
//   FILE.xslt       that DOM written as Schematron2XSLTMojo writes it
//
// The .dom format, one node per line, in document order, fields separated by a tab:
//   dom 1                               first line
//   E DEPTH URI QNAME                   element (URI empty: no namespace)
//   A DEPTH URI QNAME VALUE             attribute of the element above, in DOM order
//   T DEPTH DATA                        text
//   D DEPTH DATA                        CDATA section
//   C DEPTH DATA                        comment
//   P DEPTH TARGET DATA                 processing instruction
// DEPTH is 0 for the children of the document node. In URI, QNAME, VALUE, DATA and
// TARGET, a backslash is written \\, a tab \t, a line feed \n, a carriage return \r and
// any other character below U+0020 \x{XX} (hexadecimal).
//
// write: builds, from each FILE.xml, the DOM document Saxon's DOMWriter builds from its
// tree (an identity transformation of Saxon into a DOMResult, as step 3 makes its
// result), and writes FILE.dom and FILE.xslt into OUTDIR as compile does.
//
// skeleton: copies the stylesheets of the ISO Schematron skeleton that ph-schematron
// runs, from its class path, into OUTDIR.
//
// chars: the code units that ph-commons' XMLCharHelper (XML 1.0) takes for invalid, as
// inclusive ranges of hexadecimal code units, one line per range: "TABLE FIRST LAST",
// TABLE one of name-start, name, text, attribute-value, cdata.
//
// xerces: runs the operations of CASES (tests/data/xerces-cases.txt of the crate, whose
// header gives the format) on DOM elements of the JDK, and writes, for each case,
// "case NAME", then each attribute in DOM order ("A QNAME VALUE"), then the children
// ("T DEPTH DATA", "D DEPTH DATA", "C DEPTH DATA", "E DEPTH QNAME"), fields separated by
// a tab and escaped as in the .dom format.

import java.io.File;
import java.io.FileOutputStream;
import java.io.OutputStream;
import java.io.OutputStreamWriter;
import java.io.Writer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import javax.xml.XMLConstants;
import javax.xml.transform.ErrorListener;
import javax.xml.transform.OutputKeys;
import javax.xml.transform.Templates;
import javax.xml.transform.Transformer;
import javax.xml.transform.TransformerException;
import javax.xml.transform.TransformerFactory;
import javax.xml.transform.dom.DOMResult;
import javax.xml.transform.dom.DOMSource;
import javax.xml.transform.stream.StreamResult;
import javax.xml.transform.stream.StreamSource;

import org.w3c.dom.Attr;
import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.NamedNodeMap;
import org.w3c.dom.Node;

import com.helger.io.resource.ClassPathResource;
import com.helger.io.resource.FileSystemResource;
import com.helger.io.resource.IReadableResource;
import com.helger.schematron.saxon.SchematronTransformerFactory;
import com.helger.schematron.sch.SchematronProviderXSLTFromSCH;
import com.helger.schematron.sch.SchematronSCHConfig;
import com.helger.schematron.svrl.CSVRL;
import com.helger.xml.XMLFactory;
import com.helger.xml.XMLHelper;
import com.helger.xml.namespace.MapBasedNamespaceContext;
import com.helger.xml.serialize.write.EXMLSerializeVersion;
import com.helger.xml.serialize.write.XMLCharHelper;
import com.helger.xml.serialize.write.XMLWriter;
import com.helger.xml.serialize.write.XMLWriterSettings;
import com.helger.xml.transform.TransformSourceFactory;
import com.helger.xml.transform.XMLTransformerFactory;

public final class PhSchematronOracle
{
  /** Reports what the stylesheets report (xsl:message, warnings) on standard error. */
  private static final class Listener implements ErrorListener
  {
    public void warning (final TransformerException e)
    {
      System.err.println ("warning: " + e.getMessageAndLocation ());
    }

    public void error (final TransformerException e)
    {
      System.err.println ("error: " + e.getMessageAndLocation ());
    }

    public void fatalError (final TransformerException e) throws TransformerException
    {
      throw e;
    }
  }

  private interface Test
  {
    boolean invalid (int c);
  }

  private static Templates templates (final TransformerFactory tf, final String path)
  {
    final ClassLoader cl = SchematronProviderXSLTFromSCH.class.getClassLoader ();
    final Templates t = XMLTransformerFactory.newTemplates (tf, new ClassPathResource (path, cl));
    if (t == null)
      throw new IllegalStateException ("cannot compile " + path);
    return t;
  }

  private static void identity (final TransformerFactory tf, final Document doc, final File out) throws Exception
  {
    final Transformer t = tf.newTransformer ();
    t.setOutputProperty (OutputKeys.METHOD, "xml");
    t.setOutputProperty (OutputKeys.INDENT, "no");
    t.setOutputProperty (OutputKeys.ENCODING, "UTF-8");
    try (OutputStream os = new FileOutputStream (out))
    {
      t.transform (new DOMSource (doc), new StreamResult (os));
    }
  }

  private static String escape (final String s)
  {
    final StringBuilder b = new StringBuilder (s.length ());
    for (int i = 0; i < s.length (); i++)
    {
      final char c = s.charAt (i);
      switch (c)
      {
        case '\\' -> b.append ("\\\\");
        case '\t' -> b.append ("\\t");
        case '\n' -> b.append ("\\n");
        case '\r' -> b.append ("\\r");
        default -> {
          if (c < 0x20)
            b.append ("\\x{").append (Integer.toHexString (c)).append ('}');
          else
            b.append (c);
        }
      }
    }
    return b.toString ();
  }

  private static String uri (final String s)
  {
    return s == null ? "" : escape (s);
  }

  private static void dump (final Node node, final int depth, final Writer w) throws Exception
  {
    switch (node.getNodeType ())
    {
      case Node.ELEMENT_NODE -> {
        w.write ("E\t" + depth + "\t" + uri (node.getNamespaceURI ()) + "\t" + escape (node.getNodeName ()) + "\n");
        final NamedNodeMap attrs = node.getAttributes ();
        for (int i = 0; i < attrs.getLength (); i++)
        {
          final Attr a = (Attr) attrs.item (i);
          w.write ("A\t" +
                   depth +
                   "\t" +
                   uri (a.getNamespaceURI ()) +
                   "\t" +
                   escape (a.getNodeName ()) +
                   "\t" +
                   escape (a.getValue ()) +
                   "\n");
        }
      }
      case Node.TEXT_NODE -> w.write ("T\t" + depth + "\t" + escape (node.getNodeValue ()) + "\n");
      case Node.CDATA_SECTION_NODE -> w.write ("D\t" + depth + "\t" + escape (node.getNodeValue ()) + "\n");
      case Node.COMMENT_NODE -> w.write ("C\t" + depth + "\t" + escape (node.getNodeValue ()) + "\n");
      case Node.PROCESSING_INSTRUCTION_NODE -> w.write ("P\t" +
                                                        depth +
                                                        "\t" +
                                                        escape (node.getNodeName ()) +
                                                        "\t" +
                                                        escape (node.getNodeValue ()) +
                                                        "\n");
      default -> throw new IllegalStateException ("unexpected DOM node type " + node.getNodeType ());
    }
    for (Node child = node.getFirstChild (); child != null; child = child.getNextSibling ())
      dump (child, depth + 1, w);
  }

  private static void compile (final File outDir, final List <String> files) throws Exception
  {
    final TransformerFactory tf = SchematronTransformerFactory.getDefault ();
    final Templates step1 = templates (tf, SchematronProviderXSLTFromSCH.XSLT2_STEP1);
    final Templates step2 = templates (tf, SchematronProviderXSLTFromSCH.XSLT2_STEP2);
    final Templates step3 = templates (tf, SchematronProviderXSLTFromSCH.XSLT2_STEP3);
    for (final String file : files)
    {
      final File sch = new File (file);
      final String base = sch.getName ().replaceFirst ("\\.sch$", "");
      final IReadableResource resource = new FileSystemResource (sch);
      final ErrorListener listener = new Listener ();

      // The intermediate steps, as SchematronProviderXSLTFromSCH.createSchematronXSLT runs
      // them: DOM results, the system ID of the Schematron file on each DOM source.
      final StreamSource src1 = TransformSourceFactory.create (resource);
      final Document result1 = XMLFactory.newDocument ();
      final Transformer t1 = step1.newTransformer ();
      t1.setErrorListener (listener);
      t1.transform (src1, new DOMResult (result1));
      identity (tf, result1, new File (outDir, base + ".step1.xml"));

      final Document result2 = XMLFactory.newDocument ();
      final Transformer t2 = step2.newTransformer ();
      t2.setErrorListener (listener);
      final DOMSource src2 = TransformSourceFactory.create (result1);
      if (src2.getSystemId () == null)
        src2.setSystemId (src1.getSystemId ());
      t2.transform (src2, new DOMResult (result2));
      identity (tf, result2, new File (outDir, base + ".step2.xml"));

      // Step 3 serialized by Saxon: the result tree, without the DOM.
      final Transformer t3 = step3.newTransformer ();
      t3.setErrorListener (listener);
      t3.setOutputProperty (OutputKeys.METHOD, "xml");
      t3.setOutputProperty (OutputKeys.INDENT, "no");
      t3.setOutputProperty (OutputKeys.ENCODING, "UTF-8");
      final DOMSource src3 = TransformSourceFactory.create (result2);
      if (src3.getSystemId () == null)
        src3.setSystemId (src1.getSystemId ());
      try (OutputStream os = new FileOutputStream (new File (outDir, base + ".step3.xml")))
      {
        t3.transform (src3, new StreamResult (os));
      }

      // The whole pipeline as Schematron2XSLTMojo calls it (engine iso-schematron, no
      // phase, no language code, no parameters, no URI resolver).
      final Map <String, String> noParameters = null;
      final SchematronSCHConfig config = SchematronSCHConfig.builder (resource)
                                                            .errorListener (listener)
                                                            .uriResolver (null)
                                                            .phase (null)
                                                            .languageCode (null)
                                                            .parameters (noParameters)
                                                            .forceCacheResult (false)
                                                            .build ();
      final Document xsltDoc = SchematronProviderXSLTFromSCH.createSchematronXSLT (config);

      try (Writer w = new OutputStreamWriter (new FileOutputStream (new File (outDir, base + ".dom")),
                                              StandardCharsets.UTF_8))
      {
        w.write ("dom 1\n");
        for (Node child = xsltDoc.getFirstChild (); child != null; child = child.getNextSibling ())
          dump (child, 0, w);
      }

      writeAsMojo (xsltDoc, new File (outDir, base + ".xslt"));
      System.err.println ("compiled " + base);
    }
  }

  /**
   * Writes the DOM documents built as Saxon's DOMWriter builds them: from the tree of a
   * FILE.xml (identity), or from the result of a FILE.xsl applied to itself, whose XML
   * (Saxon's serialization, no indentation) goes into FILE.result.xml.
   */
  private static void writeFiles (final File outDir, final List <String> files) throws Exception
  {
    final TransformerFactory tf = SchematronTransformerFactory.getDefault ();
    for (final String file : files)
    {
      final File in = new File (file);
      final boolean stylesheet = in.getName ().endsWith (".xsl");
      final String base = in.getName ().replaceFirst ("\\.xml$|\\.xsl$", "");
      final Document doc = XMLFactory.newDocument ();
      try
      {
        if (stylesheet)
        {
          final Templates templates = tf.newTemplates (new StreamSource (in));
          final Transformer serializer = templates.newTransformer ();
          serializer.setOutputProperty (OutputKeys.METHOD, "xml");
          serializer.setOutputProperty (OutputKeys.INDENT, "no");
          serializer.setOutputProperty (OutputKeys.ENCODING, "UTF-8");
          try (OutputStream os = new FileOutputStream (new File (outDir, base + ".result.xml")))
          {
            serializer.transform (new StreamSource (in), new StreamResult (os));
          }
          final Transformer t = templates.newTransformer ();
          // A refused node is expected here (FILE.error): not reported.
          t.setErrorListener (new ErrorListener ()
          {
            public void warning (final TransformerException e)
            {}

            public void error (final TransformerException e)
            {}

            public void fatalError (final TransformerException e) throws TransformerException
            {
              throw e;
            }
          });
          t.transform (new StreamSource (in), new DOMResult (doc));
        }
        else
          tf.newTransformer ().transform (new StreamSource (in), new DOMResult (doc));
      }
      catch (final TransformerException e)
      {
        // The DOM refuses a node: FILE.error names the code of its DOMException.
        Throwable cause = e;
        while (cause != null && !(cause instanceof org.w3c.dom.DOMException))
          cause = cause.getCause ();
        if (cause == null)
          throw e;
        final String code = switch (((org.w3c.dom.DOMException) cause).code)
        {
          case org.w3c.dom.DOMException.INVALID_CHARACTER_ERR -> "INVALID_CHARACTER_ERR";
          case org.w3c.dom.DOMException.NAMESPACE_ERR -> "NAMESPACE_ERR";
          case org.w3c.dom.DOMException.HIERARCHY_REQUEST_ERR -> "HIERARCHY_REQUEST_ERR";
          default -> "DOMException " + ((org.w3c.dom.DOMException) cause).code;
        };
        Files.writeString (new File (outDir, base + ".error").toPath (), code + "\n");
        continue;
      }
      try (Writer w = new OutputStreamWriter (new FileOutputStream (new File (outDir, base + ".dom")),
                                              StandardCharsets.UTF_8))
      {
        w.write ("dom 1\n");
        for (Node child = doc.getFirstChild (); child != null; child = child.getNextSibling ())
          dump (child, 0, w);
      }
      writeAsMojo (doc, new File (outDir, base + ".xslt"));
    }
  }

  /** Writes the document as Schematron2XSLTMojo writes a compiled stylesheet. */
  private static void writeAsMojo (final Document xsltDoc, final File out) throws Exception
  {
    final String sNSPrefix = XMLConstants.XMLNS_ATTRIBUTE + ":";
    final Map <String, String> aMapFromXml = new LinkedHashMap <> ();
    XMLHelper.forAllAttributes (xsltDoc.getDocumentElement (), (sAttrName, sAttrValue) -> {
      if (sAttrName.startsWith (sNSPrefix))
        aMapFromXml.put (sAttrName.substring (sNSPrefix.length ()), sAttrValue);
    });
    final MapBasedNamespaceContext aNSContext = new MapBasedNamespaceContext ();
    if (!aMapFromXml.containsKey ("svrl"))
      aNSContext.addMapping ("svrl", CSVRL.SVRL_NAMESPACE_URI);
    for (final var e : aMapFromXml.entrySet ())
      aNSContext.addMapping (e.getKey (), e.getValue ());
    final XMLWriterSettings aXWS = new XMLWriterSettings ().setNamespaceContext (aNSContext)
                                                           .setPutNamespaceContextPrefixesInRoot (true);
    XMLWriter.writeToStream (xsltDoc, new FileOutputStream (out), aXWS);
  }

  /** Whether Xerces (the JDK's DOM) refuses an element named so. */
  private static boolean xercesRefuses (final Document doc, final String name)
  {
    try
    {
      doc.createElementNS (null, name);
      return false;
    }
    catch (final org.w3c.dom.DOMException e)
    {
      return true;
    }
  }

  /** Whether Xerces refuses a processing instruction with this target. */
  private static boolean xercesRefusesTarget (final Document doc, final String target)
  {
    try
    {
      doc.createProcessingInstruction (target, "d");
      return false;
    }
    catch (final org.w3c.dom.DOMException e)
    {
      return true;
    }
  }

  private static void chars (final File out) throws Exception
  {
    final EXMLSerializeVersion v = EXMLSerializeVersion.XML_10;
    final Document doc = XMLFactory.newDocument ();
    final String [] names = { "name-start",
                              "name",
                              "text",
                              "attribute-value",
                              "cdata",
                              "xerces-name-start",
                              "xerces-name",
                              "xerces-target-start",
                              "xerces-target" };
    final Test [] tests = { c -> XMLCharHelper.isInvalidXMLNameStartChar (v, c),
                            c -> XMLCharHelper.isInvalidXMLNameChar (v, c),
                            c -> XMLCharHelper.isInvalidXMLTextChar (v, c),
                            c -> XMLCharHelper.isInvalidXMLAttributeValueChar (v, c),
                            c -> XMLCharHelper.isInvalidXMLCDATAChar (v, c),
                            c -> xercesRefuses (doc, String.valueOf ((char) c)),
                            c -> xercesRefuses (doc, "a" + (char) c),
                            c -> xercesRefusesTarget (doc, String.valueOf ((char) c)),
                            c -> xercesRefusesTarget (doc, "a" + (char) c) };
    try (Writer w = new OutputStreamWriter (new FileOutputStream (out), StandardCharsets.UTF_8))
    {
      for (int t = 0; t < names.length; t++)
      {
        int first = -1;
        for (int c = 0; c <= 0x10000; c++)
        {
          final boolean bad = c <= 0xffff && tests[t].invalid (c);
          if (bad && first < 0)
            first = c;
          if (!bad && first >= 0)
          {
            w.write (String.format ("%s %04X %04X%n", names[t], first, c - 1));
            first = -1;
          }
        }
      }
      // A character outside the Basic Multilingual Plane (two code units), first and later.
      final String supplementary = new String (Character.toChars (0x10000));
      w.write ("xerces-supplementary-start " +
               (xercesRefuses (doc, supplementary) ? "invalid" : "valid") +
               "\n");
      w.write ("xerces-supplementary " + (xercesRefuses (doc, "a" + supplementary) ? "invalid" : "valid") + "\n");
      w.write ("xerces-target-supplementary-start " +
               (xercesRefusesTarget (doc, supplementary) ? "invalid" : "valid") +
               "\n");
      w.write ("xerces-target-supplementary " +
               (xercesRefusesTarget (doc, "a" + supplementary) ? "invalid" : "valid") +
               "\n");
    }
  }

  private static void xerces (final File cases, final File out) throws Exception
  {
    final List <String> lines = Files.readAllLines (cases.toPath (), StandardCharsets.UTF_8);
    try (Writer w = new OutputStreamWriter (new FileOutputStream (out), StandardCharsets.UTF_8))
    {
      Element e = null;
      for (final String line : lines)
      {
        if (line.isEmpty () || line.startsWith ("#"))
          continue;
        // Missing trailing fields are empty (an empty DATA may lose its tab).
        final String [] f = java.util.Arrays.copyOf (line.split ("\t", -1), 4);
        for (int i = 0; i < f.length; i++)
          if (f[i] == null)
            f[i] = "";
        switch (f[0])
        {
          case "case" -> {
            if (e != null)
              xercesResult (e, w);
            e = XMLFactory.newDocument ().createElementNS (null, "e");
            w.write ("case\t" + f[1] + "\n");
          }
          case "set" -> e.setAttributeNS (f[1].equals ("-") ? null : f[1], f[2], f[3]);
          case "text" -> e.appendChild (e.getOwnerDocument ().createTextNode (f[1]));
          case "cdata" -> e.appendChild (e.getOwnerDocument ().createCDATASection (f[1]));
          case "comment" -> e.appendChild (e.getOwnerDocument ().createComment (f[1]));
          case "child" -> e.appendChild (e.getOwnerDocument ().createElementNS (null, f[1]));
          case "child-text" -> {
            Node last = e.getLastChild ();
            while (last != null && last.getNodeType () != Node.ELEMENT_NODE)
              last = last.getPreviousSibling ();
            last.appendChild (e.getOwnerDocument ().createTextNode (f[1]));
          }
          case "normalize" -> e.normalize ();
          case "create" -> {
            String outcome;
            try
            {
              e.getOwnerDocument ().createElementNS (f[1].equals ("-") ? null : f[1], f[2]);
              outcome = "ok";
            }
            catch (final org.w3c.dom.DOMException x)
            {
              outcome = "error";
            }
            w.write ("create\t" + f[1] + "\t" + escape (f[2]) + "\t" + outcome + "\n");
          }
          case "attr", "target" -> {
            String outcome;
            try
            {
              final Document d = e.getOwnerDocument ();
              if (f[0].equals ("attr"))
                d.createElementNS (null, "scratch").setAttributeNS (f[1].equals ("-") ? null : f[1], f[2], "v");
              else
                d.createProcessingInstruction (f[1], "d");
              outcome = "ok";
            }
            catch (final org.w3c.dom.DOMException x)
            {
              outcome = "error";
            }
            final String rest = f[0].equals ("attr") ? f[1] + "\t" + escape (f[2]) : escape (f[1]);
            w.write (f[0] + "\t" + rest + "\t" + outcome + "\n");
          }
          default -> throw new IllegalArgumentException ("unknown operation: " + line);
        }
      }
      if (e != null)
        xercesResult (e, w);
    }
  }

  private static void xercesResult (final Element e, final Writer w) throws Exception
  {
    final NamedNodeMap attrs = e.getAttributes ();
    for (int i = 0; i < attrs.getLength (); i++)
    {
      final Attr a = (Attr) attrs.item (i);
      w.write ("A\t" + escape (a.getNodeName ()) + "\t" + escape (a.getValue ()) + "\n");
    }
    for (Node child = e.getFirstChild (); child != null; child = child.getNextSibling ())
      xercesNode (child, 0, w);
  }

  private static void xercesNode (final Node n, final int depth, final Writer w) throws Exception
  {
    switch (n.getNodeType ())
    {
      case Node.TEXT_NODE -> w.write ("T\t" + depth + "\t" + escape (n.getNodeValue ()) + "\n");
      case Node.CDATA_SECTION_NODE -> w.write ("D\t" + depth + "\t" + escape (n.getNodeValue ()) + "\n");
      case Node.COMMENT_NODE -> w.write ("C\t" + depth + "\t" + escape (n.getNodeValue ()) + "\n");
      case Node.ELEMENT_NODE -> {
        w.write ("E\t" + depth + "\t" + escape (n.getNodeName ()) + "\n");
        for (Node child = n.getFirstChild (); child != null; child = child.getNextSibling ())
          xercesNode (child, depth + 1, w);
      }
      default -> throw new IllegalStateException ("unexpected node type " + n.getNodeType ());
    }
  }

  /** Copies the stylesheets of the ISO Schematron skeleton from the class path. */
  private static void skeleton (final File outDir) throws Exception
  {
    final String dir = SchematronProviderXSLTFromSCH.SCHEMATRON_DIRECTORY_XSLT2;
    final ClassLoader cl = SchematronProviderXSLTFromSCH.class.getClassLoader ();
    for (final String name : List.of ("iso_dsdl_include.xsl",
                                      "iso_abstract_expand.xsl",
                                      "iso_svrl_for_xslt2.xsl",
                                      "iso_schematron_skeleton_for_saxon.xsl",
                                      "iso_schematron_message_xslt2.xsl"))
    {
      try (var in = new ClassPathResource (dir + name, cl).getInputStream ())
      {
        if (in == null)
          throw new IllegalStateException ("no " + dir + name + " on the class path");
        Files.write (new File (outDir, name).toPath (), in.readAllBytes ());
      }
    }
  }

  public static void main (final String [] args) throws Exception
  {
    final String mode = args.length > 0 ? args[0] : "";
    if (mode.equals ("compile") && args.length >= 3)
      compile (new File (args[1]), List.of (args).subList (2, args.length));
    else if (mode.equals ("write") && args.length >= 3)
      writeFiles (new File (args[1]), List.of (args).subList (2, args.length));
    else if (mode.equals ("skeleton") && args.length == 2)
      skeleton (new File (args[1]));
    else if (mode.equals ("chars") && args.length == 2)
      chars (new File (args[1]));
    else if (mode.equals ("xerces") && args.length == 3)
      xerces (new File (args[1]), new File (args[2]));
    else
      throw new IllegalArgumentException ("usage: PhSchematronOracle compile OUTDIR FILE.sch... | write OUTDIR FILE.xml... | skeleton OUTDIR | chars OUTFILE | xerces CASES OUTFILE");
  }
}
