<?xml version="1.0" encoding="UTF-8"?>
<!-- Synthetic: abstract patterns with parameters, abstract rules and extends, lets at
     every level. Every instance of the abstract pattern has an identifier: for one
     without, iso_abstract_expand.xsl makes one with generate-id(), which Saxon numbers
     after the documents its configuration has seen before in the same run, so that
     ph-schematron's output then depends on what it compiled earlier. -->
<schema xmlns="http://purl.oclc.org/dsdl/schematron" queryBinding="xslt2">
  <ns prefix="a" uri="urn:example:a"/>
  <let name="limit" value="100"/>
  <pattern abstract="true" id="Range">
    <rule context="$element">
      <let name="v" value="number($field)"/>
      <assert id="$id" test="$v &gt;= $min and $v &lt;= $limit">$field must be between $min and the limit.</assert>
    </rule>
  </pattern>
  <pattern is-a="Range" id="QuantityRange">
    <param name="element" value="a:Line"/>
    <param name="field" value="a:Quantity"/>
    <param name="min" value="1"/>
    <param name="id" value="R-Q"/>
  </pattern>
  <pattern is-a="Range" id="PriceRange">
    <param name="element" value="a:Line"/>
    <param name="field" value="a:Price"/>
    <param name="min" value="0"/>
    <param name="id" value="R-P"/>
  </pattern>
  <pattern id="Rules">
    <let name="currency" value="'EUR'"/>
    <rule abstract="true" id="HasCurrency">
      <assert id="R-C" test="@currency = $currency">The currency is <value-of select="$currency"/>.</assert>
    </rule>
    <rule context="a:Price">
      <extends rule="HasCurrency"/>
      <assert id="R-D" test="matches(., '^[0-9]+(\.[0-9]{1,2})?$')">At most two decimals.</assert>
    </rule>
    <rule context="a:Total">
      <extends rule="HasCurrency"/>
    </rule>
  </pattern>
</schema>
