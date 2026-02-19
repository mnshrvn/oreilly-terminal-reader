  (() => {
    const lines = ['# Netscape HTTP Cookie File'];
    document.cookie.split(';').forEach(c => {
      const [name, ...rest] = c.trim().split('=');
      const value = rest.join('=');
      if (!name) return;
      const domain = location.hostname;
      const path = '/';
      const secure = location.protocol === 'https:' ? 'TRUE' : 'FALSE';
      const expiry = Math.floor(Date.now() / 1000) + 86400 * 365;
      const domainFlag = domain.startsWith('.') ? 'TRUE' : 'FALSE';
      lines.push(`${domain}\t${domainFlag}\t${path}\t${secure}\t${expiry}\t${name}\t${value}`);
    });
    const text = lines.join('\n') + '\n';
    copy(text);
    console.log('Cookies copied to clipboard in Netscape format!\n\n' + text);
  })();