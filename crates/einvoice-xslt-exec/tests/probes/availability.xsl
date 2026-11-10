<?xml version="1.0" encoding="UTF-8"?>
<!-- element-available, function-available (with and without arity, a stylesheet function,
     EXSLT's node-set, an unknown namespace) and system-property, evaluated at run time. -->
<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:f="urn:f"
    xmlns:exsl="http://exslt.org/common" xmlns:u="urn:u" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xsl:function name="f:mine"><xsl:param name="a"/><xsl:sequence select="$a"/></xsl:function>
  <xsl:template match="/">
    <out ea1="{element-available('xsl:namespace')}" ea2="{element-available('xsl:import-schema')}"
        ea3="{element-available('exsl:document')}" ea4="{element-available('xsl:template')}"
        fa1="{function-available('concat')}" fa2="{function-available('concat', 1)}"
        fa3="{function-available('f:mine')}" fa4="{function-available('f:mine', 2)}"
        fa5="{function-available('exsl:node-set')}" fa6="{function-available('u:none')}"
        fa7="{function-available('xs:decimal', 1)}" fa8="{function-available('id')}"
        fa9="{function-available('generate-id', 0)}" fa10="{function-available('f:mine', 1)}">
      <p n="version" v="{system-property('xsl:version')}"/>
      <p n="vendor" v="{system-property('xsl:vendor')}"/>
      <p n="vendor-url" v="{system-property('xsl:vendor-url')}"/>
      <p n="product-name" v="{system-property('xsl:product-name')}"/>
      <p n="product-version" v="{system-property('xsl:product-version')}"/>
      <p n="is-schema-aware" v="{system-property('xsl:is-schema-aware')}"/>
      <p n="supports-serialization" v="{system-property('xsl:supports-serialization')}"/>
      <p n="supports-backwards-compatibility" v="{system-property('xsl:supports-backwards-compatibility')}"/>
      <p n="supports-namespace-axis" v="{system-property('xsl:supports-namespace-axis')}"/>
      <p n="supports-streaming" v="{system-property('xsl:supports-streaming')}"/>
      <p n="supports-dynamic-evaluation" v="{system-property('xsl:supports-dynamic-evaluation')}"/>
      <p n="supports-higher-order-functions" v="{system-property('xsl:supports-higher-order-functions')}"/>
      <p n="xpath-version" v="{system-property('xsl:xpath-version')}"/>
      <p n="xsd-version" v="{system-property('xsl:xsd-version')}"/>
      <p n="other" v="{system-property('xsl:other')}"/>
      <p n="u" v="{system-property('u:vendor')}"/>
    </out>
  </xsl:template>
</xsl:stylesheet>
