#!/usr/bin/env python3
"""Independent UTM reference (Krüger n-series, Karney 2011), WGS-84.

Shares no code with src/mgrs.rs (Snyder series); use it to produce reference
values when changing the MGRS/UTM code:

    python3 -I scripts/utm_reference.py            # built-in points
    python3 -I scripts/utm_reference.py LAT LON ZONE
"""
import math, sys
a=6378137.0; f=1/298.257223563; k0=0.9996
n=f/(2-f); A=a/(1+n)*(1+n*n/4+n**4/64+n**6/256)
al=[None, n/2-2*n*n/3+5*n**3/16+41*n**4/180-127*n**5/288+7891*n**6/37800,
    13*n*n/48-3*n**3/5+557*n**4/1440+281*n**5/630-1983433*n**6/1935360,
    61*n**3/240-103*n**4/140+15061*n**5/26880+167603*n**6/181440,
    49561*n**4/161280-179*n**5/168+6601661*n**6/7257600,
    34729*n**5/80640-3418889*n**6/1995840,
    212378941*n**6/319334400]
def utm(lat, lon, zone):
    lon0=math.radians((zone-1)*6-180+3)
    phi=math.radians(lat); lam=math.radians(lon)-lon0
    e=math.sqrt(f*(2-f))
    t=math.sinh(math.atanh(math.sin(phi))-e*math.atanh(e*math.sin(phi)))
    xi=math.atan2(t, math.cos(lam)); eta=math.atanh(math.sin(lam)/math.sqrt(1+t*t))
    x=eta+sum(al[j]*math.cos(2*j*xi)*math.sinh(2*j*eta) for j in range(1,7))
    y=xi+sum(al[j]*math.sin(2*j*xi)*math.cosh(2*j*eta) for j in range(1,7))
    E=500000+k0*A*x; N=k0*A*y + (10000000 if lat<0 else 0)
    return E,N
points = [(float(sys.argv[1]), float(sys.argv[2]), int(sys.argv[3]))] if len(sys.argv) == 4 else [(38.8895,-77.0353,18),(0,0,31),(45,3,31),(56.9637,24.2289,35),(-33.8568,151.2153,56),(60,5,32),(78,15,33)]
for lat,lon,zone in points:
    E,N=utm(lat,lon,zone); print(f"{lat},{lon},{zone},{E:.3f},{N:.3f}")
